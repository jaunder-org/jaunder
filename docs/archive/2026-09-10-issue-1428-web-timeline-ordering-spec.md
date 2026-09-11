# Issue #1428: Viewer-selectable web Post timeline ordering

## Outcome

Every web Post timeline presents Posts in a chronology that agrees with the
publication timestamp shown to the viewer. A viewer can switch between Newest
and Oldest without leaving the timeline, and the URL records that choice.

## Load-bearing decisions

- The ordering contract applies to every web Post timeline:
  - the public site timeline at `/`;
  - the authenticated home timeline at `/app`;
  - public User timelines;
  - public site-tag timelines; and
  - public User-tag timelines.
- “Web Post timeline” is the scoped term for this feature. It does not broaden
  **Syndication Feed**, which remains reserved for RSS, Atom, and JSON Feed in
  `CONTEXT.md`.
- Chronology is the Post's publication timestamp—the timestamp displayed on its
  card—not its internal creation timestamp. A later-published Post therefore
  appears above an earlier-published Post in Newest order even if it was created
  first.
- Newest orders by publication timestamp descending, then Post ID descending.
  Oldest is the exact reverse: publication timestamp ascending, then Post ID
  ascending. Post ID is the sole deterministic tie-break.
- Newest is the canonical default. Its URL has no ordering query parameter.
  `?order=oldest` selects Oldest. An absent or unknown `order` value renders
  Newest rather than failing the page.
- Ordering state belongs only to the URL. It is not stored in an account,
  cookie, browser storage, or site configuration.
- Each affected timeline presents the same compact sort-direction icon button
  immediately above the Post list. Its accessible name and tooltip state both
  the active order and the order the action will select.
- Activating the button performs same-origin SPA navigation to the opposite
  order's URL, replaces the current rows with that order's first page, and
  resets continuation state. Browser history can return to the prior order.
- Pagination remains keyset-based and opaque. A continuation cursor is bound to
  the ordering that produced it; pages from opposite orders cannot be mixed.
  Each order remains stable at equal timestamps through its Post ID tie-break.
- Publication-time edits may reposition a Post relative to an in-progress
  pagination walk. The no-omission/no-duplicate guarantee applies while the
  ordered data set is unchanged; this feature does not add snapshot isolation
  across concurrent publication-time mutations.
- The public projector honors the URL order when producing the initial page and
  seed. Projector output and CSR rendering remain coincident, so direct loads do
  not repaint into a different order after mount. The representation remains
  anonymous and cacheable by its complete URL.
- Visibility, publication eligibility, scheduled go-live, deletion filtering,
  page-size limits, and theme ownership remain unchanged.
- This contract is recorded architecturally in
  `docs/adr/0190-web-post-timeline-ordering.md` because it changes the keyset
  cursor's time axis and adds URL-addressed projector variants.

## Acceptance

- Given Posts whose creation order differs from their displayed publication
  times, every affected timeline shows them by displayed publication time.
- Newest shows later publication timestamps first; Oldest shows earlier
  publication timestamps first.
- Equal publication timestamps are deterministic by Post ID, descending for
  Newest and ascending for Oldest.
- With an unchanged ordered data set, SQLite and PostgreSQL return identical
  first and continuation pages for both orders, without omissions or duplicates
  at equal-timestamp or ordinary page boundaries.
- Every affected timeline exposes the accessible sort-direction icon button,
  reflects the active URL state, and starts again from the first page when
  toggled.
- The canonical Newest URL omits `order`; selecting Oldest yields
  `?order=oldest`; an unknown value displays Newest.
- Loading an Oldest public timeline URL directly paints Oldest from the
  projector and remains in that order when CSR mounts.
- Loading more preserves the selected order. A stale request or cursor from the
  previous order cannot append rows after the viewer switches.
- Supplying a Newest cursor to an Oldest request, or an Oldest cursor to a
  Newest request, is rejected without returning a mixed page.
- Back and forward navigation restore the URL-selected order and matching rows.
- After selecting Oldest, loading any affected timeline's bare URL in the same
  browser—including while authenticated—still yields canonical Newest; no
  account, site, cookie, or browser state remembers the prior selection.
- Existing authorization and visibility behavior remains identical in both
  orders, including the anonymous projector and authenticated home timeline.
- Existing Syndication Feed, AtomPub Collection, drafts, and management-list
  ordering remains unchanged.

## Boundaries

- No Author grouping or Author sort is added.
- No account-level, site-level, cookie, or local-storage default is added.
- No sort applies to RSS, Atom, JSON Feed, AtomPub, drafts, media, audience,
  subscription, or administrative listings.
- No page-number navigation, random order, relevance order, filtering, or
  configurable secondary key is introduced.
- The issue does not change Post timestamps or which timestamp a Post card
  displays; it makes timeline ordering agree with that existing display.
