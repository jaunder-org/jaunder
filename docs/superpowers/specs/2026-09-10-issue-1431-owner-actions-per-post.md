# Issue #1431: Place owner actions with their Post

## Outcome

After login, each Post owned by the current User presents one compact Actions
control as part of that Post's header. Multiple owned Posts no longer produce
repeated action trays in the page topbar, and the detached-header failure
reported in #1433 is resolved by the same change.

## Load-bearing decisions

- Every owned Post exposes its own Actions button; other Users' Posts do not.
  Its menu contains Edit, History, Publish/Unpublish, and Delete for that exact
  Post.
- The button appears visually at the right side of the Post header. The menu
  replaces the previously proposed always-visible row: one fixed-size trigger
  remains legible without making narrow headers wrap four mutation controls.
- The button and menu retain accessible names, keyboard operation, focus
  behavior, and an exact relationship to their Post. The redundant visible
  `Actions for @username` tray title is removed.
- The controls remain trusted client-side decoration outside the custom-theme
  Style Contract surface. Theme Package CSS cannot directly style, reorder, or
  suppress the button or menu, and anonymous projector markup remains
  viewer-independent. A theme that removes or clips the Post/header ancestor can
  indirectly remove its visual anchor; switching that public presentation to
  Studio is the supported recovery path for mutation controls.
- Every viewer-independent Post header contains a small in-flow reserved slot
  sized for the Actions button. Jaunder-owned, inline-important declarations
  protect that slot's own principal box, dimensions, and unique anchor
  association from direct Theme Package rules. The empty slot is present for
  anonymous viewers, so adding owner controls after mount changes no Post
  geometry.
- Declarative CSS Anchor Positioning tethers the trusted sibling button to its
  Post's reserved slot and the trusted menu to its button. The DOM/layout order
  must make each themed anchor acceptable before its positioned trusted control.
  No scroll, resize, or geometry-measurement JavaScript fallback is introduced.
- The supported owner-control layout requires the basic CSS anchor-positioning
  surface available in Chromium 125+, Firefox 147+ (including ESR 153+), and
  Safari/iOS 26+. Older browsers are outside this authenticated mutation-UI
  contract; anonymous reading remains unaffected.
- Chromium and Firefox remain the required CI browser matrix. Before landing,
  the exact cross-surface interaction also receives a one-time Playwright WebKit
  smoke run. If that runner cannot provide reliable evidence for the relevant
  Safari engine behavior, landing instead requires a simple reproducible manual
  Safari 26 check and its observed result.
- Existing mutation behavior, authorization, confirmation, navigation, feedback,
  and Post publication semantics remain unchanged.
- The trusted global region remains available for genuinely global warnings. It
  no longer presents Post-specific controls as global chrome.
- This browser compatibility and trusted-overlay mechanism is recorded in
  `docs/adr/0186-trusted-post-actions-use-css-anchors.md` and projected into
  `docs/ARCHITECTURE.md`. It does not change Jaunder's domain vocabulary.

## Acceptance

- On `/`, `/app`, a User timeline, and a Post permalink, every visible owned
  Post has exactly one accessible Actions button aligned with that Post's
  header; no button appears for another User's Post.
- Activating a button opens exactly one trusted menu associated with that Post.
  Edit, History, Publish/Unpublish, and Delete retain their current observable
  behavior, including confirmation, feedback, and navigation.
- Pointer and keyboard users can open and dismiss the menu, traverse every item,
  and return focus to its trigger. Escape and outside activation dismiss it
  without dispatching an action.
- A page containing several owned Posts has no Post action tray in the page
  topbar, no repeated global `Actions for` labels, and no ambiguity about which
  Post an open menu controls.
- Chromium and Firefox desktop screenshots show a cohesive result for the #1431
  and #1433 cases. A narrow-viewport exercise shows the reserved trigger slot
  and open menu without overlap, clipping, horizontal overflow, or content
  movement.
- Owner decoration causes no movement of anonymous Post header or body geometry
  across CSR mount in both gated browsers.
- A custom Theme Package that broadly styles visible Post descendants cannot
  directly style or suppress the reserved anchor association, trusted button, or
  trusted menu. If a theme removes the Post/header ancestor, switching to Studio
  restores the mutation controls.
- A one-time Playwright WebKit smoke exercises multiple Posts, menu interaction,
  scrolling, and narrow placement. An explicit manual Safari 26 result replaces
  it if the WebKit runner proves unreliable for this mechanism.
- Signed-out public markup and first paint remain byte-identical per URL.

## Boundaries

- No always-visible mutation row, selection/focus model, bulk Post actions, or
  global contextual toolbar.
- No JavaScript layout coordinator and no compatibility fallback for browsers
  without CSS Anchor Positioning.
- No changes to mutation endpoints, authorization policy, Post data, or routing.
- No general redesign of Post typography, display-name handling (#1432), or
  non-Post topbar warnings.
- #1433 is combined only to the extent that removing the detached global action
  tray makes the Post header one visual unit; unrelated header styling is out of
  scope.
