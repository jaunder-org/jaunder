# ADR-0044: Authenticated-owner flash-free enhancement (pre-paint marker + additive decoration)

- Status: accepted
- Date: 2026-07-02
- Issue: [#181](https://github.com/jaunder-org/jaunder/issues/181)

## Context

ADR-0041 established the public projector + leptos-CSR client: the projector
emits **byte-identical anonymous HTML per URL** (that is _why_ it is
CDN-cacheable) and explicitly deferred the authenticated leg — _"Per-viewer /
authenticated enhancement is a client concern (#181), never the projector's."_
This ADR records that leg.

The hard boundary (`docs/hub-architecture.md` §4): a **cacheable,
anonymously-rendered** public page cannot know the visitor is the owner, so the
SPA must adjust post-boot. The naive approaches both flash: an **async** "am I
logged in?" call guarantees paint-then-swap, and **re-rendering a different
DOM** for the owner discards the server's paint. The current Sidebar does
exactly the former (async `current_user()` as the auth source, with an anonymous
`<Suspense>` fallback), and `PostDisplay`'s authored branch does the latter (a
different-DOM reactive article than the projector painted). Both must go.

## Decision

1. **The pre-paint signal is a JS-readable localStorage _auth marker_ —
   advisory, not a credential.** A small `{username}` JSON in localStorage. It
   is never sent to the server, so the projector stays byte-identical/cacheable
   with **no CDN config and no server change**, and it is synchronously readable
   before paint. The real session remains the existing HTTP-only cookie the
   `#[server]` fns use; the marker only says "probably the owner — pre-adjust
   chrome." A new device / incognito has no marker → the safe anon default.
   (Rejected: a CDN-ignored cookie — sent every request, so projector _and_ CDN
   must both strip it from the cache key; cache-poisoning risk for no pre-paint
   benefit.)

2. **A tiny inline, blocking `<head>` script sets `<html class="authed">` before
   first paint.** It reads the marker and marks the document (plus the username)
   so CSS **pre-adjusts / reserves the authed layout** and the SPA boots already
   knowing. Inline + blocking (not external, not deferred) — a network
   round-trip would defeat pre-paint. It appears on **both** HTML surfaces: the
   projector's `document()`/`render_head` (cacheable public pages) and
   `csr/index.html` (the SPA-shell fallback). One Rust constant is the single
   source of truth; a unit test asserts `csr/index.html` contains it (drift
   guard). The bytes are identical for everyone → cacheability intact.

3. **The marker is the boot source; `current_user()` is a background
   reconcile.** Components render authed/anon chrome **synchronously from the
   marker** (no `<Suspense>` gate on first paint). The async `current_user()`
   still runs, now only as a correctness backstop: it confirms the marker (happy
   path); corrects a dead-session marker **toward anon** (rare, and the safe
   direction); or adds chrome when the marker is absent but the session is live
   (an uncommon edge, no longer the norm). The marker is **written on login,
   cleared on logout, and corrected on a reconcile mismatch.**

4. **Enhancement is _additive decoration on the untouched DOM_, never a branch
   switch.** The projector-painted content DOM (article `inner_html`, anonymous
   sidebar `inner_html`) stays byte-identical; owner affordances are **new
   elements layered on** — the own-post action column as a CSS-positioned
   sibling/overlay revealed by `html.authed` + author-matches-marker; the authed
   sidebar nav/footer appended into a **reserved slot**. The shared pure render
   fn reserves any needed slot so both sides still coincide. Flash-freeness =
   **coincidence** (anon parts render identical → no visual change on the
   delete-`#app`-and-remount of ADR-0041) **+ reserved-space additive fill**
   (authed chrome fills CSS-reserved slots → no reflow; chrome fades in).
   (Rejected: a marker-driven branch switch — still a different-DOM article for
   own-posts → localized flash; and unifying authored posts onto `inner_html` —
   previously regressed the authed home-feed e2e, the action column needs
   reactive handlers.)

5. **The cockpit is a distinct route (`/app`), never an enhancement of a public
   page — and `/` stays the enhanced public timeline for the owner.** A
   personalized feed is _different content_ than the projector's cacheable
   anonymous paint, so it can never coincide; making `/` viewer-aware would
   break cacheability. Therefore `/` stays the public Local timeline for the
   owner (enhanced with own-post affordances + authed sidebar — the Local→Feed
   swap is removed from `home.rs`), and the personalized home **Feed relocates
   to the cockpit at `/app`** (the existing Feed branch: composer +
   `list_home_feed`). `/app` is a first-class, directly-bookmarkable authed
   route (served from the SPA shell, `html.authed` pre-painted → boots straight
   into the cockpit). At `/`, the owner **stays** on the enhanced front page by
   default; the synced redirect preference is deferred (pre-paint read path in
   place, defaulting to stay — never redirect on a guess). The richer cockpit
   (read-state, inline drafts, nav hub) is future (sync engine).

## Consequences

- **Cacheability is untouched.** The projector still renders anonymous-only,
  byte-identical per URL; every per-owner adjustment is client-side
  (localStorage
  - CSS + the CSR client). Acceptance's "anonymous responses remain
    byte-identical/cacheable" holds by construction.
- **The marker can be stale** (dead cookie / cleared storage). The reconcile
  bounds the blast radius: a wrong "authed" corrects toward anon (safe), a
  missing marker on a live session degrades to the old async-add (uncommon).
  Tightening this (e.g. a session-expiry in the marker) is deliberately
  deferred.
- **Render-fn drift is the flash risk.** A coincidence unit test (projector
  output vs. the shared fn, including reserved slots) is the structural guard;
  e2e adds a synchronous pre-paint `authed`-class assertion + affordance
  presence. An empirical layout-shift (CLS) assertion is a deferred,
  flakiness-prone follow-on.
- **Unblocks #182.** The final authed UX is in place before the parallel-e2e
  (`workers>1`) campaign, per the chosen ordering.
- Extends ADR-0041; the marker/enhance vocabulary graduates into
  `docs/hub-architecture.md` §8 (auth marker, pre-paint auth detection).

## Addendum (2026-07-22): shared session context + operator in the marker (#591)

Decision 1's marker was `{username}`; it now carries `{username, is_operator}`.
The extra field is additive — the pre-paint `<head>` script (Decision 2) reads
only `.username`, so `username` stays the top-level key and the script, its
`csr/index.html` twin, and the drift guard are unchanged. Decode treats an
absent `is_operator` as `false`, so a marker written before this change decodes
as a (non-operator) logged-in session rather than anonymous — no flash for
in-flight sessions. The value type is `web::auth::SessionUser`.

**Operator chrome is now flash-free too.** Previously the operator-only sidebar
links were gated on an async `current_user_is_operator()` fetch (a
paint-then-add flash for operators). With `is_operator` in the marker the
sidebar seeds it synchronously at boot, like the username. It stays advisory:
the operator-gated `#[server]` fns still call `require_operator()` against the
DB, so a hand-edited marker only reveals a link that rejects on use.

**Decision 3's reconcile is lifted to one app-level context.** The per-component
`current_user()` fetches (sidebar, cockpit, create-post, subscribe-button) and
the reactive `current_user_is_operator()` are retired in favour of a single
`session()` `#[server]` fn returning `Option<SessionUser>`, exposed through a
shared `SessionContext` provided in `AppShell`: a marker-seeded
`RwSignal<Option<SessionUser>>` plus a per-navigation reconcile `Resource` that
confirms the marker against the live session and rewrites both the signal and
the marker. Components read the context (synchronous seed for chrome; awaited
reconcile for server-confirmed gates) rather than each spinning their own fetch
— the "boot from marker, reconcile in the background, correct toward anon"
mechanism of Decision 3 is unchanged, just consolidated into the single source.

**The full-reload write path is gone.** Decision 3 says the marker is "written
on login, cleared on logout." That write used to ride a `set_redirect_hook`
override that forced every server-fn redirect into a full `location.replace()`
document load (an SSR-era vestige — SSR was later removed, ADR-0041/#180). #591
deletes the override: login/logout/register now update the shared context
(signal + marker) directly and navigate via `leptos_router`'s client-side
pushState hook — no wasm re-boot. See the #591 spec for the full rationale.
