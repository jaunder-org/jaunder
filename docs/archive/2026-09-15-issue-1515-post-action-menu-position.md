# Issue #1515: Keep permalink Post Actions attached to their trigger

## Outcome

On an authenticated owner's canonical Post permalink, opening Post Actions
places the menu immediately below and visually attached to the Actions trigger.
The menu remains fully usable and inside the viewport at ordinary and narrow
browser widths instead of appearing at the left side of the screen.

## Load-bearing decisions

- Preserve ADR-0188's trusted Post Actions architecture: the viewer-independent
  Post header retains its reserved anchor slot, while the trigger and menu
  remain portalled into the trusted sibling outside the Theme Package surface.
- Continue to use CSS Anchor Positioning within ADR-0188's supported-browser
  contract. Do not add JavaScript geometry reads, observers, resize or scroll
  handlers, or an older-browser positioning fallback.
- Treat the menu as belonging to the visible Actions trigger: its top edge is
  below the trigger with no more than an 8 CSS-pixel gap, and its right edge
  aligns with the trigger's right edge within 1 CSS pixel. Viewport clamping may
  override edge alignment only when alignment would otherwise put part of the
  menu outside the viewport.
- Preserve the existing action set, authorization, native popover dismissal, and
  focus-restoration behavior.
- Preserve the projector/CSR coincidence and additive owner-enhancement
  contracts: the fix must not move Post content, create a visible layout shift,
  or change anonymous permalink presentation.

## Acceptance

- A browser regression test enters an authenticated owner's canonical Post
  permalink and opens Actions at both an ordinary 1280 × 720 viewport and a
  narrow 375 × 800 viewport.
- At both sizes, the test proves the menu starts below the trigger with a gap
  from 0 through 8 CSS pixels and that their right edges differ by no more than
  1 CSS pixel unless viewport clamping is required.
- At both sizes, the test proves all four edges of the complete menu bounding
  box remain inside the viewport, rather than allowing the menu to detach at or
  beyond the viewport's left edge.
- The permalink menu continues to expose Edit, History, Publish or Unpublish as
  appropriate, and Delete.
- Native outside-click and Escape dismissal and trigger focus restoration remain
  covered by the Post Actions behavior suite.
- Existing multi-Post and cross-route Post Actions behavior remains green.
- Anonymous permalink markup remains free of trusted Post Action controls.

## Boundaries

- No redesign of the Post header, menu contents, or action workflows.
- No change to Theme Package capabilities or the trusted-sibling boundary.
- No expansion of the browser compatibility floor established by ADR-0188.
- No generalized popover framework or unrelated form/menu styling work.
