# Clarify Local and Home surfaces

Issue: #1449

## Outcome

Jaunder presents two unambiguous timeline surfaces: **Local** at `/` is the
public, viewer-independent landing timeline, while **Home** at `/app` is the
authenticated User’s own publishing cockpit. A browser carrying Jaunder’s auth
marker goes from `/` to Home before Local content paints.

## Load-bearing decisions

- Keep `/` and `/app` as distinct routes. Their public/cacheable and
  authenticated/private responsibilities remain separate.
- **Local** is the canonical user-facing name for `/`. It contains currently
  published, non-deleted public Posts originating on this instance, regardless
  of who requests it.
- Local’s projector seed and client-side continuation requests resolve the same
  viewer-independent Post set. Authentication may add owner controls and other
  chrome but must not add audience-restricted rows to Local.
- **Home** is the canonical user-facing name for `/app`. It remains an
  authenticated timeline of the current User’s own published Posts with the
  inline composer; it is not expanded into a followed-source or read-state
  timeline.
- Bare **Feed** is removed from navigation and Home page copy. **Syndication
  Feed** retains its existing RSS, Atom, and JSON meaning and discovery links.
- A structurally valid auth marker triggers a blocking pre-paint redirect from
  `/` to `/app`. The public projector remains anonymous and cacheable; it does
  not inspect session state or emit viewer-specific bytes.
- The auth marker remains advisory rather than authoritative. `/app` confirms
  the real session before fetching Home rows; a stale marker redirects through
  `/app` and then to `/login` under the existing unauthenticated behavior.
- A live session with no auth marker cannot redirect before the public projector
  paints. The background reconcile restores the marker and immediately replaces
  Local with `/app`; this one recovery path is the explicit exception to the
  pre-paint guarantee.
- A root request with the recognized `order=oldest` state redirects to
  `/app?order=oldest`. Missing, newest, or malformed ordering redirects to the
  canonical newest-first `/app` URL; unrelated query parameters are not carried
  across.
- Authenticated navigation exposes Home and does not offer Local. Anonymous
  navigation exposes Local and does not offer Home.
- Login success, registration success, and authenticated brand/root navigation
  target `/app` directly. Same-document navigation must not depend on the
  document-level pre-paint script running again.
- The unused Home redirect preference is removed. Redirect-to-Home is the one
  policy, with no compatibility key, preference UI, or opt-out path.
- The
  [authenticated-root redirect decision](../../adr/drafts/authenticated-root-redirects-home.md)
  supersedes ADR-0044’s stay-on-`/` default and deferred redirect preference
  while retaining its cacheability, additive-enhancement, and
  session-confirmation boundaries.
- Repository domain and architecture documentation define Local and Home with
  the same meanings as the UI.

## Acceptance

- A signed-out visit to `/` renders Local and can paginate only public Posts
  from local Users, in the existing URL-selected publication order.
- A valid auth marker at `/` redirects before Local content paints, including on
  a direct or bookmarked visit.
- `/?order=oldest` redirects to `/app?order=oldest`; absent, newest, malformed,
  and unrelated query state redirects to `/app`.
- A stale auth marker follows `/` → `/app` → `/login` without exposing Home data
  or entering a redirect loop.
- A live session with no marker may paint Local once, then reconciliation
  restores the marker and replaces the route with `/app`; subsequent root loads
  redirect before paint.
- Login success, registration success, and authenticated brand/root navigation
  reach `/app` directly without rendering Local.
- `/app` remains authentication-gated and shows only the current User’s
  published Posts plus the inline composer; Posts from other Users do not
  appear.
- Signed-out navigation labels `/` as Local. Authenticated navigation labels
  `/app` as Home and contains no Local or bare Feed item.
- Home chrome describes the surface without using bare Feed terminology.
- Local’s projected first page and client-side fetches use the same public-only
  visibility contract for both SQLite and PostgreSQL-backed reads.
- Syndication Feed endpoints, discovery metadata, and labels retain their
  existing behavior.
- The new redirect decision and architecture view explicitly identify ADR-0044
  Decision 5 as superseded while retaining its cacheability,
  additive-enhancement, and session-confirmation boundaries; the design guide,
  flow documentation, and domain glossary agree with the delivered Local/Home
  contract.

## Boundaries

- No combined public/private timeline and no viewer-aware projector output.
- No followed-source aggregation, inbound-reading timeline, read state, inline
  drafts, or richer cockpit work.
- No storage schema, audience-policy, Post lifecycle, timeline ordering,
  pagination, profile, tag, AtomPub Collection, or Syndication Feed change.
- No redirect preference, Local opt-out, or durable authenticated access to
  Local.
