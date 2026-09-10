# ADR-0186: Trusted Post Actions Use CSS Anchor Positioning

- Status: accepted
- Date: 2026-09-10
- Issue: [#1431](https://github.com/jaunder-org/jaunder/issues/1431)

## Context

[ADR-0044](0044-authenticated-owner-flash-free-enhancement.md) requires owner
affordances to be additive decoration over viewer-independent public markup.
[ADR-0184](0184-css-package-public-themes.md) additionally places mutation
controls in a trusted stacking context that is a sibling of the custom-theme
surface, so owner-authored CSS cannot hide or restyle them.

The first implementation satisfied those trust boundaries by portalling every
owned Post's action tray into one page-level top-right region. A timeline with
several owned Posts therefore accumulates several detached trays over the page,
and even one tray looks unrelated to the Post it controls. Putting the controls
directly inside the themed Post header would fix visual ownership while
violating the Theme Package isolation boundary.

The controls need to remain separate in the document and coincide with their
Post visually, without a layout-measurement loop that tracks scrolling,
resizing, font loading, and content changes in JavaScript. A full external
action row cannot participate in the themed header's flow, so it cannot wrap
reliably without a larger protected Style Contract region.

## Decision

Each viewer-independent Post header contains a compact in-flow slot reserved for
one Actions button. Jaunder-owned inline-important declarations protect the
slot's own principal box, fixed dimensions, and unique `anchor-name` from direct
Theme Package rules. A theme may arrange the header around that real box, but it
cannot directly remove the box or the association. Because the slot exists in
anonymous markup, owner enhancement reserves no new space after mount.

The Actions button and its menu are mounted through a minimal Portal into the
dedicated `#j-trusted-post-actions` sibling, before warning-only
`#j-trusted-chrome`. CSS Anchor Positioning associates the button with exactly
one Post slot and the menu with its trigger. Layout/source order ensures each
themed anchor is acceptable before its positioned trusted control. The button
appears over its reserved slot; the menu exposes Edit, History,
Publish/Unpublish, and Delete without putting mutation controls inside the theme
surface.

The implementation uses only the basic interoperable anchor surface:
`anchor-name`, `position-anchor`, and `anchor()`-based placement. It does not
depend on newer fallback-order or visibility features, nor on JavaScript
geometry, scroll, resize, or font-load coordination.

This authenticated owner-control surface requires Chromium 125 or later, Firefox
147 or later (including ESR 153 or later), or Safari/iOS 26 or later. Browsers
without CSS Anchor Positioning are outside the mutation-UI compatibility
contract; anonymous reading remains viewer-independent and unaffected. We prefer
an explicit modern-browser floor to a second stateful positioning
implementation.

Chromium and Firefox remain the required CI browser matrix. The feature receives
a one-time Playwright WebKit smoke before landing, covering the exact
cross-surface association, scrolling, menu interaction, and narrow layout. If
that runner cannot reliably represent the relevant Safari engine behavior, a
simple reproducible manual Safari 26 check and recorded result replace it.

The warning-only `#j-trusted-chrome` sibling remains the owner of global
warnings, while `#j-trusted-post-actions` owns only Post-specific controls.
Action behavior and server authorization remain unchanged.

## Consequences

Mutation controls retain the trusted sibling boundary while appearing to belong
to the Post they mutate. Custom public themes can arrange visible Post
presentation around the protected reserved slot but cannot directly style or
suppress the trusted button or menu. A theme that removes or clips the
Post/header ancestor indirectly removes the visual anchor; switching that public
presentation to Studio is the supported recovery path for mutation controls.
This deliberately narrows ADR-0184's unconditional reachability consequence
while preserving its direct CSS isolation. The projector/CSR coincidence
contract and zero-shift additive enhancement remain intact.

Every Post header reserves one compact button footprint for anonymous and owner
views. That small permanent cost replaces both the detached global trays and the
larger reservation an always-visible action row would require.

The application acquires an explicit browser-version floor for authenticated
Post mutation UI. Firefox ESR 140 and Safari/iOS before 26 do not receive that
surface. Cross-engine browser evidence must exercise multiple owned Posts,
scrolling, narrow placement, menu behavior, and custom-theme isolation.

The mechanism avoids runtime geometry reads, observers, and scroll handlers, but
it depends on interoperable CSS anchor behavior across the supported engines.
Expanding compatibility to older browsers would require a separately justified
fallback architecture rather than an incidental script in this change.
