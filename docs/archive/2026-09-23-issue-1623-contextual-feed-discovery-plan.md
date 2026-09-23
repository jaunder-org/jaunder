# Contextual Syndication Feed Discovery — Implementation Outline (#1623)

> Execute with `jaunder-iterate`; use `jaunder-dispatch` only for an
> independently bounded task. This outline exists because four new public web
> routes must preserve the ADR-0041 projector/CSR seed and presentation
> contract.

## Scope

In: contextual RSS markers on four public timelines, four bookmarkable discovery
pages, existing three-format Syndication Feed URLs, tests, and transient
before/after visual proof.

Out: new feeds or serializers, AtomPub Collection links, feed directory, private
pages, new Style Contract hooks, or feed/autodiscovery changes.

## Task outline

- [x] Capture pre-change Studio visual baselines for Local, a User timeline, and
      the nearest containing shell for a discovery route at desktop and compact
      sizes as applicable.
  - Contract: save transient route, viewport, theme, account/fixture, and
    capture paths; compare the finished UI under those same conditions.
  - Verification: baseline images exist before any presentation mutation and are
    reproducible for final comparison.
- [x] Define and project the typed discovery context and four public
      destinations.
  - Contract: `PageSeed` distinguishes a discovery destination from a timeline
    and carries its `FeedSurface` context; canonical feed links derive from
    `common::feed::canonicalize`, not duplicated URL templates. Server projector
    and CSR consume the same pure, non-reactive markup for the anonymous shell
    and page (ADR-0041). New nested routes do not capture existing timeline,
    permalink, or feed paths.
  - Verification: host/HTTP tests cover all four direct routes, three canonical
    format links per context, empty valid timelines, unknown User profile versus
    unknown User-tag fallback, malformed path fallback, seed round-trip, and
    unchanged feed/profile RSD autodiscovery.
- [x] Add the contextual marker and client-side discovery navigation.
  - Contract: marker derives its destination from the same typed timeline
    context, appears only in the public navigation-rail footer, and has the
    accessible name “Syndication feeds”; authenticated `/` still redirects
    before Local paints. No marker appears on a Post permalink, Home, or other
    private pages.
  - Verification: focused browser tests cover all four markers and discovery
    pages, direct and in-app navigation, accessible name, empty and missing
    cases, no boot/navigation flash, and marker absence. Capture final
    Local/User pairs and a discovery-page image; compare with baselines for
    unintended differences.

## Risk checks

- Keep the existing `<link rel="alternate">` syndication autodiscovery and User
  profile `rel="EditURI"` unchanged; the new page lists no AtomPub Collection.
- Do not route authenticated `/` to a rendered public Local page, expose
  viewer-specific projector markup, or introduce a reactive server render.
- Use the existing web vertical file split (ADR-0070), soft-path fallback
  conventions, and one-boot Playwright navigation discipline.
- Focused host/browser proofs precede commits; review the deliverable before
  broad validation and PR preparation. Merge requires separate explicit
  approval.
