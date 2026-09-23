# Compact timeline sort control

Issue: [#1624](https://github.com/jaunder-org/jaunder/issues/1624)

## Outcome

Every web Post timeline presents its sort-direction control as a compact
masthead action instead of reserving a full-width row between the page chrome
and Posts. The control remains easy to find, accessible, and behaviorally
identical while recovering the wasted vertical space.

## Load-bearing decisions

- Apply the placement consistently to all five web Post timelines: Local, Home,
  User, site-tag, and User-tag.
- Place the control in the existing right-aligned masthead action area.
- On Local, the anonymous Sign in and conditional Register actions retain their
  source order, followed by the sort control. The existing masthead action area
  may wrap at narrow widths, but every action remains visible and operable with
  no clipping, overlap, or horizontal page overflow.
- On Home, the control belongs inside the Home topbar; the inline composer
  remains a separate following sibling below the masthead and does not own
  timeline ordering.
- User and tag timelines use the same masthead-action placement rather than a
  route-specific variant.
- Preserve ADR-0190's ordering contract: Newest remains the canonical default,
  Oldest remains URL-addressed by `order=oldest`, and toggling restarts the
  timeline from its first page.
- Preserve the existing icon, tooltip, accessible name, focus treatment, and
  active-direction semantics.
- Public projector markup and the CSR presentation remain coincident: the
  control is already present in the projected masthead and does not move or
  duplicate when the client mounts.
- This is a presentation refinement, not a new Style Contract guarantee or a
  change to Theme Package authority.

## Acceptance

- `/`, `/app`, `/~alice`, `/tags/rust`, and `/~alice/tags/rust` each show one
  sort control in the page masthead and no standalone timeline-order row.
- Activating the control from Newest navigates within the app to the same route
  with `order=oldest`; activating it from Oldest returns to the canonical bare
  route.
- The control's accessible name and tooltip state the current order and the
  action that activation will take.
- Local renders Sign in, conditional Register, then sort in masthead source
  order. At both the desktop proof viewport and 390 × 844 CSS pixels, each
  action remains visible and operable; wrapping is permitted, but clipping,
  overlap, and horizontal page overflow are not.
- Home renders the sort control structurally inside its masthead action area;
  the composer remains outside and immediately after the masthead.
- Projected public timelines and their mounted CSR equivalents retain the
  existing no-layout-shift guarantee.
- Existing timeline-order behavioral coverage passes, with regression coverage
  pinning the masthead placement and absence of the standalone row.
- Visual proof compares before and after at a desktop Local route with anonymous
  actions, a desktop authenticated Home route with its composer, and Local at
  390 × 844 CSS pixels, using identical data, theme, and authentication state
  per pair.

## Boundaries

- Do not change Post ordering keys, cursor encoding, storage queries, timeline
  pagination, or URL parsing.
- Do not add another ordering mode or persist a User preference.
- Do not redesign the masthead, composer, authentication actions, or Post cards.
- Do not change Syndication Feed, AtomPub Collection, drafts, or management-list
  ordering.
- Do not add a new ADR; ADR-0190 already owns the timeline-ordering contract and
  this work stays within it.
