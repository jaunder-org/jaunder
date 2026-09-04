# Composer Discoverability and Sidebar Active State

Issue: #896

## Outcome

Authenticated authors can reach the full `/posts/new` composer through a
persistent Compose item in the sidebar. The sidebar also identifies the linked
destination whose exact path is currently displayed.

## Load-bearing decisions

- The full composer remains a first-class advanced authoring surface. The
  `/posts/new` route, `CreatePostPage`, and its custom-slug, scheduling,
  audience, named-audience, format, media, summary, and tag controls remain.
- The `/app` inline composer remains the fast authoring path; it does not
  replace or gain the full composer's controls in this change.
- The authenticated sidebar gains a linked item with key `compose`, label
  `Compose`, href `/posts/new`, and the existing Edit icon.
- Compose appears after Feed and before the draft-management destinations. It is
  hidden from anonymous sidebar navigation under the existing authentication
  policy.
- Active navigation is derived centrally from the current pathname and the
  sidebar catalog. A linked item is active only when its href exactly equals the
  current pathname.
- Exact matching intentionally makes Home active only at `/`, Feed only at
  `/app`, Compose only at `/posts/new`, and each management/operator item only
  at its catalog href.
- Routes without their own sidebar destination, including individual Post,
  editor, revision-detail, and tag routes, leave every sidebar item inactive.
  They do not inherit a parent highlight through prefix matching.
- The same catalog-derived active decision drives every authenticated linked
  destination; Compose does not receive a one-off route special case.
- Anonymous navigation remains limited to its existing public destinations. At
  `/`, Home receives the same exact-path active treatment. The anonymous
  projector must emit that active state before hydration; projected non-root
  public routes remain inactive so projector and reactive sidebar markup stay
  coincident.
- Navigation uses an ordinary anchor and the existing in-app navigation helper
  in browser tests; no synthetic test-only route transition is introduced.

## Acceptance

- An authenticated sidebar displays Compose after Feed, linking to `/posts/new`,
  with the established sidebar item markup and Edit icon.
- An anonymous sidebar does not display Compose.
- Activating Compose through the rendered sidebar reaches the full composer
  without a document reload and exposes its New post / Long-form surface.
- `/posts/new` marks Compose active and leaves every other sidebar item
  inactive.
- Each existing linked sidebar destination is active on its exact catalog href,
  including operator destinations when visible.
- `/`, `/app`, and `/drafts` prove that active styling changes as in-app
  navigation changes the current pathname.
- A route without an exact sidebar href leaves every item inactive.
- Host tests cover catalog order, authentication/operator filtering, exact
  pathname-to-active-key derivation including Home and `/posts/new`, and the
  projector's root-versus-non-root active decision.
- Projector and anonymous reactive sidebar markup agree before hydration: Home
  is already active for projected `/`, while projected non-root public routes
  contain no active sidebar item.
- End-to-end coverage proves the real Compose affordance and reactive active
  state. Every existing second-hop navigation to `/posts/new` that is no longer
  a cold first entry uses the rendered Compose link through the established
  in-app navigation helper; true first-entry composer tests may still load the
  route directly.

## Boundaries

- No composer form, payload, validation, authentication, audience, publication,
  scheduling, or storage behavior changes.
- No route removal, redirect, URL alias, topbar CTA, or duplicate compose link.
- No prefix-based grouping of nested routes under sidebar destinations.
- No redesign of sidebar layout, labels, icons, sources, or footer.
