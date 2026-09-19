# Compact Home composer and unified scrolling

**Issue:** #1578

## Outcome

Home (`/app`) presents a compact inline composer followed by the User's
published Posts in one natural page flow. Desktop keeps the Home header and
composer available while Posts scroll beneath them; mobile gives the composer
normal flow so the full viewport remains useful.

Home has one browser/document vertical scrollbar. Page Down, keyboard scrolling,
mouse-wheel scrolling, and touch scrolling move through the same continuous Home
surface instead of becoming trapped in a separately scrolling Posts region.

## Load-bearing decisions

- This is a Home-specific presentation change. Local and other timeline screens
  retain their existing scrolling behavior.
- Home uses one browser/document vertical scroll surface; neither the composer
  nor the Posts region owns an independent vertical scrollbar.
- In the wide, two-column composer layout, the compact Home header and composer
  remain fixed/sticky at the top while the ordering control and Posts move
  beneath them.
- Sticky Home chrome yields to normal document flow while a composer disclosure
  is expanded or an operational warning banner is visible. This keeps every
  field reachable and prevents sticky surfaces from overlapping without
  introducing a second scrollbar.
- When the composer changes to its narrow, stacked layout, it returns to normal
  document flow rather than occupying a sticky share of the smaller viewport.
- The sticky behavior follows the composer's existing wide-versus-stacked layout
  transition rather than a separate device classification.
- The composer is content-sized. It does not reserve a fixed percentage or large
  minimum share of the viewport.
- All existing composer disclosure controls and actions remain immediately
  visible. Their field contents retain the existing expand/collapse behavior.
  Compactness comes from eliminating imposed empty height and tightening
  presentation, not from hiding or removing capabilities.
- The ordering control remains part of the Posts stream. It is not pinned with
  the composer.
- Sticky chrome has an opaque, correctly layered surface so Posts may pass
  beneath it without showing through or obscuring controls. A visible
  operational warning remains unobscured and causes the Home chrome to use
  normal flow.
- Home retains its existing Post ordering, cursor pagination, publication
  behavior, and authentication boundaries.

## Acceptance

- At a 1440×900 viewport with enough seeded Posts to scroll, Home shows a
  content-sized two-column composer without the former large dead-space band.
- At that desktop viewport, scrolling leaves the compact Home header and
  composer available while the ordering control and Posts move beneath them;
  expanding a disclosure returns the chrome to normal flow and keeps its
  complete field body reachable.
- At a 390×844 viewport, the composer is stacked in normal flow and the User can
  scroll from every composer control into the Posts using the page's sole
  vertical scrollbar.
- On both representative viewports, the Posts region and composer have no
  independent vertical scrollbar.
- Page Down from Home advances through the Posts rather than only moving an
  inner Posts pane.
- Every existing inline-composer disclosure control and action remains visible,
  and every field remains reachable and usable through its existing disclosure
  behavior, at both representative viewports.
- The ordering control remains immediately before the Post list in the scrolling
  stream, and loading additional Posts preserves the single-scroll behavior.
- Comparable before/after screenshots at 1440×900 and 390×844 demonstrate the
  changed density, sticky behavior, and mobile flow with equivalent seeded data.
- Automated browser coverage proves the desktop sticky state, the narrow normal-
  flow state, the absence of nested vertical scrolling, and continued ordering
  and pagination behavior.

## Boundaries

- Do not change Local or other timeline screens merely to share Home's scrolling
  model.
- Do not redesign the composer, change its fields, or alter Post creation
  semantics.
- Do not replace the existing ordering control or cursor-based pagination.
- Do not introduce automatic infinite loading as part of this work.
- Do not update visual-regression baselines solely to document this issue; the
  before/after evidence is transient review material.
