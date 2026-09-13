# ADR-DRAFT: Authenticated root visits redirect to Home

- Status: proposed
- Date: 2026-09-12
- Issue: [#1449](https://github.com/jaunder-org/jaunder/issues/1449)

## Context

Jaunder has two timeline routes with different presentation boundaries. `/` is a
publicly projected, cacheable timeline of public Posts originating on the
instance. `/app` is an authenticated cockpit containing the current User's own
published Posts and inline composer.
[ADR-0044](../0044-authenticated-owner-flash-free-enhancement.md) separated
those routes to keep viewer-specific content out of the public projector, but
retained `/` for authenticated owners by default and reserved an unused local
redirect preference.

Calling `/` “Home” and `/app` “Feed” hid this distinction. It also overloaded
“Feed,” which Jaunder otherwise uses for RSS, Atom, and JSON Syndication Feeds.
Keeping both routes while leaving authenticated Users on the public landing page
makes the primary publishing cockpit secondary and requires maintaining an
unused preference seam.

The existing advisory auth marker is available to the blocking pre-paint script.
It can select the authenticated route without making the public projector
viewer-aware. Because the marker can outlive the server session, `/app` remains
the authority that confirms authentication before returning private data.

## Decision

`/` is **Local**, the public, viewer-independent landing timeline. Its projected
and client-fetched rows contain only currently published public Posts from local
Users. Authentication may add owner controls to public rows, but does not change
which rows Local contains.

`/app` is **Home**, the authenticated User's own publishing cockpit. It retains
the User's published Posts, inline composer, authentication gate, ordering, and
pagination. It is not a Syndication Feed or a broader followed-source reading
timeline.

A structurally valid auth marker at `/` causes the blocking pre-paint script to
redirect to `/app`. Recognized `order=oldest` state becomes `/app?order=oldest`;
absent, newest, malformed, and unrelated query state becomes the canonical
`/app`. The `/app` session reconcile remains authoritative, so a stale marker
continues to `/login` without fetching Home data.

A live server session with no marker necessarily receives the public projection
before the client can identify it. The background session reconcile restores the
marker and immediately replaces Local with `/app`; subsequent document loads
take the pre-paint path. Login success, registration success, and authenticated
brand/root navigation target `/app` directly rather than relying on the
document-level script to run during same-document navigation.

Anonymous navigation exposes Local. Authenticated navigation exposes Home and
not Local. Bare Feed wording is removed from this UI. The unused Home redirect
preference key and read path are deleted; redirect-to-Home is the sole policy.

This decision supersedes only
[ADR-0044](../0044-authenticated-owner-flash-free-enhancement.md) Decision 5's
stay-on-`/` default and deferred redirect preference. ADR-0044's
public-projector cacheability, advisory-marker, pre-paint, additive-decoration,
and server-confirmation boundaries remain in force.

## Consequences

- Auth-marked visitors reach Home without painting Local first. A live session
  missing its marker may paint public Local once before reconciliation restores
  the marker and replaces the route with Home.
- A stale marker can cause `/` → `/app` → `/login`; this is the bounded cost of
  using an advisory pre-paint signal rather than flashing public content while
  awaiting session confirmation.
- Authenticated Users have no Local navigation or opt-out route; the public
  Local URL consistently redirects when their marker is present.
- Login, registration, and authenticated brand navigation target Home directly;
  they never use Local as an authenticated transition route.
- Local's client-side reads must use anonymous visibility resolution so they
  remain coincident with the projected first page.
- The existing redirect-preference mechanism has no remaining owner and is
  removed rather than retained as dormant compatibility surface.
- Syndication Feed endpoints and discovery metadata are unchanged.
