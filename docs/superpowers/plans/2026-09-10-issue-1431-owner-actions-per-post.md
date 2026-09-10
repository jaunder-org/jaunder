# Trusted per-Post Actions Implementation Outline

> Execute with `jaunder-iterate`, delegating slices through `jaunder-dispatch`.
> This outline exists because the trusted overlay crosses the custom-theme
> boundary and establishes a browser-compatibility floor.

## Scope

In:

- Replace global Post action trays with one trusted, anchored Actions menu per
  owned Post.
- Preserve viewer-independent Post geometry, custom-theme isolation, existing
  mutation behavior, and global warning chrome.
- Close the shared #1431/#1433 presentation failure with Chromium, Firefox, and
  one-time WebKit evidence.

Out:

- Mutation API, authorization, Post data, routing, display-name work, bulk
  actions, JavaScript geometry fallback, and permanent WebKit CI expansion.

## Task outline

- [x] Task 1: Establish the protected per-Post anchor contract
  - Contract: `posts::render::post_action_anchor_name(PostId) -> String` is the
    one host/wasm constructor for `--j-post-actions-<decimal-post-id>`. Shared
    Post markup emits one compact in-flow slot per Post with that identity;
    Jaunder-owned inline-important declarations own the slot's principal box,
    dimensions, and `anchor-name`. Trusted chrome follows an acceptable
    layout/source order for cross-surface anchors while global warning behavior
    remains unchanged.
  - Verification: host renderer tests prove stable viewer-independent slots and
    unique associations; a custom-theme fixture with broad descendant rules
    cannot directly override the protected slot properties; signed-out
    projector/CSR coincidence remains intact. A focused browser assertion proves
    backup/site warnings remain reachable and visually above the themed surface
    after the trusted-chrome reorder.

- [x] Task 2: Replace each action tray with an accessible trusted disclosure
  - Depends on: Task 1 and its `post_action_anchor_name` constructor.
  - Contract: one trusted Actions `<button>` consumes the shared Post anchor and
    controls one native auto popover containing ordinary Edit/History links and
    Publish/Unpublish/Delete buttons. The trigger/popover use their native
    disclosure relationship and exposed expanded state; the container is a
    labelled group, not an ARIA `menu` with unimplemented roving-arrow
    semantics. Native light-dismiss/Escape closes it, completed selection closes
    it, and focus returns to the trigger unless navigation replaces the page.
    Opening a trigger closes any other auto popover. Existing mutation dispatch
    paths stay authoritative. A theme-hidden/clipped Post/header may remove the
    visual anchor, with Studio as the documented recovery path.
  - Verification: focused browser flows cover own-versus-other Posts, multiple
    Posts, pointer and Tab-key interaction, Escape/outside dismissal,
    focus-return, every existing mutation path, and absence of Post trays from
    global chrome on `/`, `/app`, a User timeline, and a Post permalink. Assert
    the trigger's machine-exposed expanded/controlled relationship and run
    `expectAccessible(page)` while the popover is open.

- [x] Task 3: Add selectable browsers to the owned local E2E lifecycle
  - Depends on: none; may be implemented before Task 2, but Task 4 consumes it.
  - Contract: `cargo xtask e2e-local` gains a local-only
    `--browser chromium|firefox|webkit` selector whose default remains Chromium.
    It reuses the existing build, ephemeral server/database, capture,
    panic-verification, and teardown lifecycle while selecting the matching
    ordinary Playwright project. Snapshot-update mode keeps its existing
    Chromium+Firefox behavior and rejects the selector. This does not add WebKit
    to CI or the authoritative backend/browser matrix.
  - Verification: xtask plan/CLI tests pin default compatibility, each project
    selection, filter forwarding, snapshot-mode conflict, and honest lifecycle
    failure. Run one existing lightweight browser-flow spec through each local
    selector so the slice proves all three complete lifecycles before the
    owner-actions scenario consumes them.

- [x] Task 4: Prove cross-engine placement and finish the architecture cutover
  - Depends on: Tasks 1-3.
  - Contract: use only `anchor-name`, `position-anchor`, and `anchor()`; no
    geometry observers/listeners. Update the existing owner-flash/CLS and visual
    contracts rather than retaining selectors or prose for the global tray. Keep
    the accepted ADR, architecture projection, Style Contract guidance, and
    issue #1433 disposition consistent with the delivered behavior.
  - Verification: focused `cargo xtask e2e-local` runs prove desktop and narrow
    Chromium placement; the new Firefox selector gives the same focused signal,
    and `devtool run -- cargo xtask e2e sqlite firefox` supplies the
    repository's existing hermetic Firefox proof. The focused WebKit selector is
    the one-time reliability smoke. These flows cover multiple Posts, scrolling,
    open-menu accessibility, global warnings, custom-theme direct isolation,
    zero Post geometry shift, and the Studio recovery path: apply a theme that
    removes or clips the Post/header, observe the unavailable anchored control,
    switch the presentation to Studio through supported UI, and observe the
    correct control restored. If host WebKit cannot establish reliable evidence,
    provide a single-command Safari 26 manual check and record its observed
    result before landing. Reviewed Chromium/Firefox screenshots cover
    #1431/#1433. Each task still certifies with
    `devtool run -- cargo xtask check` before commit.

## Risk checks

- Anchor names are unique and derived without exposing a second Post identity
  convention.
- The reserved slot exists in anonymous markup and does not appear only after
  authentication.
- Theme Package CSS cannot directly style the trusted trigger/menu or override
  the slot's own protected declarations; Studio remains the recovery path when a
  theme removes/clips an ancestor.
- Anchor acceptability, nested scrolling, paint containment, stacking, focus,
  clipping, and narrow placement are exercised in every supported engine for
  which support is claimed.
- Global backup/site warnings remain reachable and visually above the themed
  surface after Post-specific controls leave their container.
- Every `TrustedPostActions`, `.j-post-action-tray`, and global-tray test/doc
  reference is migrated or removed; obsolete tray/title CSS and aggregation code
  do not survive. Only minimal Portal transport into the dedicated trusted
  actions sibling remains.
- `CONTEXT.md` remains unchanged because this introduces no domain term.
