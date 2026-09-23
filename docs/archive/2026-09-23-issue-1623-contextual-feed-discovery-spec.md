# Issue #1623 — Contextual Syndication Feed discovery

## Outcome

Visitors can find a visible, familiar RSS marker on each public timeline that
has a Syndication Feed. Activating it opens a contextual page with direct RSS,
Atom, and JSON Feed links for that timeline.

## Load-bearing decisions

- The marker belongs to Local (`/`), site-tag (`/tags/<tag>`), User
  (`/~<username>`), and User-tag (`/~<username>/tags/<tag>`) timelines only. A
  single Post permalink does not have a contextual feed marker. Home and other
  private pages are not feed-discovery surfaces.
- Place one icon-only standard RSS marker in the public navigation-rail footer.
  On compact screens, that rail becomes the top bar and the marker remains at
  its right edge. Give the link the accessible name **Syndication feeds**;
  navigate in the same tab. It is public chrome, visible to signed-in and
  signed-out viewers whenever that public timeline renders; authenticated `/`
  continues redirecting to Home before Local paints.
- The discovery destination is a public, directly bookmarkable page nested under
  the current timeline's route:
  - `/feeds` for Local;
  - `/tags/<tag>/feeds` for a site tag;
  - `/~<username>/feeds` for a User;
  - `/~<username>/tags/<tag>/feeds` for a User tag.
- Each destination identifies the exact timeline context and lists only its
  existing RSS, Atom, and JSON Feed URLs as ordinary labeled links. Keep the
  page minimal: no directory of Users or tags, copy controls, format advice, or
  AtomPub Collection links. The destination and marker remain available even
  when that timeline has no Posts. A valid but unused tag is an empty timeline;
  unknown User routes follow their corresponding timeline's existing
  projection/fallback semantics rather than inventing a new discovery context.
  Invalid path values retain the existing soft-shell fallback.
- Feed URLs, formats, cache behavior, eligibility, and existing invisible
  `<link rel="alternate">` discovery remain unchanged. The User profile's
  AtomPub RSD `rel="EditURI"` autodiscovery remains unchanged and is not listed
  on these pages. A discovery page is a human-facing index of existing public
  Syndication Feeds, not a new feed.
- Projector and CSR consume one typed, seeded feed-surface context and share
  pure non-reactive presentation for the anonymous shell and destination
  (ADR-0041). A direct load works without JavaScript; boot and client-side
  navigation must not flash a different marker or target.

## Acceptance

- On each of the four timeline shapes, a keyboard-accessible, screen-reader-
  named RSS marker opens its matching nested discovery page. On permalinks,
  Home, and other non-feed pages, no contextual marker appears. Signed-in
  visitors to public profile/tag timelines see the same marker and destination.
- Every discovery page loads directly and via client-side navigation, names its
  context, and presents three working links whose URLs and formats match that
  timeline's existing RSS, Atom, and JSON Feed representations; no unrelated
  feed is listed. The same is true for an empty timeline.
- Host/HTTP and browser tests cover the four contexts, direct navigation,
  seeded/client coincidence, accessible marker, empty case, and the absence on a
  Post permalink. Test an unknown User's empty profile and the shell fallback
  for an unknown User-tag separately. Existing feed and profile RSD
  autodiscovery continue to work.
- Comparable transient Studio-theme before/after captures show Local and a
  representative User timeline at 1440×900 and 390×844, including the
  navigation-rail footer/top-bar placement. A destination capture shows its
  contextual heading and three links. Only intended chrome and route changes
  appear in the comparison.

## Boundaries

- No new persisted state, feed endpoint or serializer, authoring workflow,
  subscription UI, invitation flow, or site-wide legal/footer content.
- Do not create a global feed directory, enumerate every User/tag, attach a
  marker to individual Posts, or add a new stable Style Contract hook solely for
  the RSS affordance.
