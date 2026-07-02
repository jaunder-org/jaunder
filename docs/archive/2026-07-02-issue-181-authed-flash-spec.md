# Spec — Issue #181: Authenticated-owner flash handling (enhance-don't-replace + pre-paint auth)

- **Issue:** jaunder-org/jaunder#181
- **Milestone:** Off concurrent SSR (web re-architecture v1) — _stabilization_
- **Date:** 2026-07-02
- **Design source:** `docs/hub-architecture.md` §4 ("Flash-free for the
  authenticated owner"); extends ADR-0041 (which forward-referenced the
  authenticated leg to #181), implements ADR-0040 direction.
- **Records:** the design interview (grill-with-docs). This spec is a _record of
  what was resolved_, not a task list — the plan (`jaunder-plan`) sequences the
  work.

> **Blocker note.** #182 (re-enable parallel e2e, `workers>1`) declares
> `blocked-by: #181`. The user chose to land #181 first so the parallel-e2e
> campaign runs against the _final_ authed UX. #181 is itself unblocked (its
> dependency #179, the leptos-CSR client, merged in PR#192).

## Goal & boundary

Make the **authenticated owner** flash-free on **cacheable, anonymously-rendered
public pages**. The one hard boundary (§4): the server emits _identical bytes
for every anonymous visitor_ (that is _why_ the page is CDN-cacheable), so it
cannot know the visitor is the owner — the SPA must adjust post-boot **without a
flash or reflow**. Anonymous responses stay byte-identical/cacheable; every
per-owner adjustment is client-side only.

Two disciplines, both from §4, resolved into concrete mechanisms below:
**enhance-don't-replace** and **pre-paint auth detection**.

## Resolved decisions

### D1 — Auth marker: a JS-readable **localStorage** advisory presence marker

The pre-paint signal is a value in **localStorage**, not a cookie. It is never
sent to the server, so the projector stays byte-identical/cacheable with **zero
CDN config and zero server change**, and it is synchronously readable in a
blocking inline script. A brand-new device / incognito session has no marker →
the safe `anon` default (§4). Rejected: a CDN-ignored cookie (sent on every
request → projector _and_ CDN must both strip it from the cache key; more moving
parts, cache-poisoning risk, no pre-paint benefit).

### D2 — Marker contents: `{username}`

The marker holds a tiny JSON object carrying the **username**. Presence → the
owner is (probably) authed; the value lets the footer-avatar/identity chrome
paint immediately instead of popping in after an async fetch. **It is not a
credential** — the real session remains the existing HTTP-only cookie the
`#[server]` fns already use; any XSS that could read the marker can already read
the session. One field to keep coherent.

### D3 — Marker = boot source; `current_user()` = reconcile

Components read auth **synchronously from the marker at boot** and render their
authed/anon chrome immediately — **no `<Suspense>` gate on first paint** (the
current Sidebar's async-`current_user()`-as-source-with-anon-fallback _is_ the
paint-then-swap we are removing). The existing async `current_user()` server-fn
still runs, now purely as a **background reconcile / correctness backstop**:

- happy path → it confirms the marker, no change;
- marker says authed but the session is dead → corrects **toward anon** (rare,
  post-expiry; the _safe_ direction — stop showing owner UI that shouldn't
  show);
- marker absent but session live → the old async-add path, now an uncommon edge
  (cleared localStorage / restored cookies), not the norm.

Marker lifecycle: **written on successful login, cleared on logout, corrected on
a `current_user()` reconcile mismatch.**

### D4 — Enhance mechanism: **additive decoration on untouched DOM**

The projector-painted content DOM (post article `inner_html`, anonymous sidebar
`inner_html`) stays **byte-identical**; owner affordances are **new elements
layered on**, never a switch to a different-DOM reactive branch:

- the own-post **action column** becomes a CSS-positioned sibling/overlay,
  revealed when `html.authed` **and** the post's author matches the marker
  username;
- the authed **sidebar nav / footer avatar** are appended into a **reserved
  slot**.

The shared pure render fn reserves any slot it needs so **both** the projector
and the CSR client still coincide. Rejected: (a) a marker-driven _branch switch_
(still renders a different article DOM for own-posts → localized flash on
exactly the owner's content); (b) unifying authored posts onto `inner_html`
(tried before, regressed the authed home-feed e2e — the action column needs
reactive handlers `inner_html` can't carry).

Interaction with the existing mount model: ADR-0041's mount deletes `#app` and
remounts fresh; flash-freeness comes from **coincidence** (anon parts render
byte-identical → no visual change) plus **reserved-space additive fill** (authed
chrome fills CSS-reserved slots → no reflow; chrome "fades in"). No change to
the delete-and-remount model.

### D5 — Pre-paint script: inline, blocking, in `<head>`, on **both** surfaces

A tiny **inline, blocking** `<script>` in `<head>` reads the marker and sets
`<html class="authed">` (+ username data) **before first paint**, so CSS
pre-adjusts (reserves the authed layout) and the SPA boots already knowing. It
is **not** external (a network round-trip defeats pre-paint) and **not**
deferred. It appears on **both** HTML surfaces:

- the projector's `document()` / `render_head` (the cacheable public pages), and
- `csr/index.html` (the static SPA-shell fallback for authed-only routes /
  uncacheable content).

**Single source of truth:** the script text is one Rust constant in the render
layer; a unit test `include_str!`s `csr/index.html` and asserts it contains the
same text (drift guard). The script bytes are identical for everyone →
cacheable.

The pre-paint `html.authed` class only _reserves layout / sets authed CSS mode_;
the authed chrome **content** still appears when the wasm client mounts, but
into pre-reserved space → no reflow.

### D6 — Cockpit at `/app`: hosts the **relocated home Feed**

A distinct authed-only route **`/app`** (clear vs. `/` the public front page;
`/home` rejected for colliding with the site home). It establishes the §4
invariant (**the cockpit is never an enhancement of a public page**, keeping
every public page a pure enhance-case) and is the redirect target the deferred
preference will point at.

**Its landing content is the personalized home Feed relocated off `/`** (see
D10) — the existing `home.rs` **Feed branch**: `InlineComposer` +
`list_home_feed` + `PostCard`s + load-more. This is real, existing code moved to
its proper home, not new work. `/app` is a **first-class, directly-bookmarkable
route**: hit directly it serves the SPA shell (`no-store`), the pre-paint script
sets `html.authed`, the client boots knowing it's the owner, and the router
renders the cockpit with **zero clicks** — so an owner who wants the feed as
their landing just bookmarks `/app`. An anonymous/expired visit to `/app` gets
the standard authed-route bounce to `/login` (as `/drafts` does today). A
_richer_ cockpit (read-state, inline drafts, nav hub to the other authed pages)
is future (sync engine) → **deferred**; in #181 the cockpit is the relocated
Feed. **No re-nesting** of the other authed routes under `/app` (future).

### D10 — `/` stays the enhanced public timeline for the owner (no Local→Feed swap)

Discovered while grounding the plan: `home.rs` today seeds the anonymous
**Local** timeline from the projector (flash-free), then `current_user()` swaps
`timeline_mode` **Local → Feed** for the owner — a **content swap** (different
posts, topbar, hero→composer), _not_ chrome decoration. A content swap
**cannot** be made flash-free by coincidence (the personalized feed is different
data than the cacheable anonymous paint; making the projector viewer-aware would
break cacheability). Therefore, for `/` to be flash-free for the owner, **`/`
must stay the public Local timeline**, enhanced with owner affordances (own-post
edit/delete action columns + authed sidebar) — the Local→Feed swap is
**removed** from `home.rs`, and the Feed relocates to the cockpit (D6). The
owner **stays on the enhanced front page by default** (§4); the deferred
preference (D7) redirects to `/app`, and bookmarking `/app` reaches it directly
regardless. The enhanced `/` carries **no** `InlineComposer` — compose lives at
`/app` with the Feed.

### D7 — `/` stay-vs-redirect preference: safe default now, redirect+sync deferred

§4's `/` preference is a _synced_ user setting with a locally-cached copy the
pre-paint script reads; the sync engine (§6) does not exist yet. #181 ships the
pre-paint script **reading a localStorage redirect-pref key that nothing writes
yet** → always the safe **stay/enhance** default. The redirect code path exists
and is exercised (a test sets the key), so acceptance-#3 ("preference works
pre-paint with the safe default") is literally satisfied. The user-facing toggle

- cross-device sync land with the sync engine → **deferred (follow-on issue)**.

### D8 — Verification: layered (coincidence unit test + pre-paint/affordance e2e)

- **Rust unit test:** projector output vs. the shared render fn stay
  byte-coincident, **including** any reserved decoration slots (the structural
  guard against render-fn drift reintroducing a flash).
- **e2e:** for an authed owner, assert `document.documentElement` has the
  `authed` class **synchronously** (before wasm/network) — proves pre-paint
  detection — then assert owner affordances (action column, authed sidebar)
  become present.
- **Not** attempting brittle pixel/CLS/reflow diffing here: the
  additive-plus-reserved-space design makes reflow structurally impossible, and
  the coincidence test guards the structure. An empirical layout-shift assertion
  is a **possible follow-on** (see below) — its timing/browser flakiness is
  exactly the kind of thing that would undermine #182's parallel-e2e stability.

### D9 — Record as **new ADR-0043**

A sibling ADR, _"Authenticated-owner flash-free enhancement (pre-paint marker +
additive decoration)."_ ADR-0041 explicitly deferred this leg to #181, and the
decisions here (the advisory-marker security/cacheability model, the
additive-decoration enhance mechanism, the cockpit-route invariant) are
substantial enough for their own record + `docs/README.md` row. **Number:** 0042
is claimed by #160/PR#195 (org→atom), which merges first; write **0043** and
reconcile at ship (rebase onto main; if 0042 is somehow absent, renumber).

## Scope

**In #181:** the localStorage marker + write/clear/reconcile wiring; the
pre-paint inline script on both surfaces + drift guard; additive-decoration
enhancement of the _existing_ authed chrome (own-post action column via the
`render_post_content` inner*html seam, authed sidebar nav/footer) on public
pages; the `html.authed` CSS reserve-layout rules; **removing the Local→Feed
swap from `home.rs` so `/` stays the enhanced public timeline** (D10); the
**`/app` cockpit route hosting the relocated home Feed** (D6); the pre-paint
redirect-pref \_read* path with the stay default; the coincidence unit test +
pre-paint/affordance e2e; ADR-0043 + glossary + README row.

**Out (deferred, filed as follow-on issues in plan task 1):**

1. **Synced `/` redirect preference** — the user-facing toggle UI + cross-device
   sync of the preference (depends on the §6 sync engine).
2. **Empirical layout-shift (CLS) e2e assertion** — a Playwright
   bounding-box/CLS check that content doesn't move between first paint and
   post-mount; _possible follow-up_, explicitly noting the
   timing/browser-flakiness downside.
3. **Rich cockpit surface** — owner feed / read-state / inline drafts at `/app`
   (depends on the sync engine); optionally re-nesting authed routes under
   `/app`.

## Acceptance (from the issue) → how it's met

- _No flash/reflow on cacheable pages; content stays, affordances appear without
  repaint_ → D4 additive decoration + D5 reserved-space pre-paint (D8 guards
  it).
- _Auth state known before first paint (no async-detect swap)_ → D1/D2/D3/D5.
- _Cockpit is a distinct route; `/` preference works pre-paint with safe
  default_ → D6 (`/app`) + D7 (stay default via pre-paint read path).
- _Anonymous responses remain byte-identical/cacheable_ → D1 (localStorage,
  never sent) + D4 (projector still renders anonymous-only; enhancement is
  client-side).
