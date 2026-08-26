# Root-relative web links implementation outline

> Execute with dev-cycle-iterate. This outline exists because two agents can
> implement independent sidebar and post slices only if they share the same
> framework-boundary contract.

## Scope

In:

- Typed sidebar catalog, operator destinations, and `SidebarNavItem::href`.
- Optional typed `PostView` permalink and typed `PostCard` edit destination.
- Focused regression coverage for rendered links and absent permalinks.

Out:

- New domain types, `RootRelativeUrl` representation or validation changes, and
  unrelated URL adoption.
- Route, authorization, visibility, or rendering-copy changes.

## Task outline

- [x] Task 1: Retain typed URLs through shared sidebar rendering
  - Contract: the shared catalog owns `Option<RootRelativeUrl>` initialized
    once; `SidebarNavItem` receives `Option<&RootRelativeUrl>`; Maud and Leptos
    conversions occur only at their anchor attributes. Operator-only
    destinations use the same one-time typed initialization. No common-crate API
    change.
  - Verification: focused `web` sidebar tests prove anonymous and authenticated
    href output, active classes, non-link placeholders, and operator
    destinations remain unchanged.

- [x] Task 2: Retain typed URLs through post rendering
  - Contract: `PostView::permalink` is `Option<&RootRelativeUrl>` at every
    construction site; absent stays `None`; `PostCard` creates a
    `RootRelativeUrl` from its typed `PostId` route and converts only at the
    Leptos anchor. No symbols or files from Task 1 are consumed.
  - Verification: focused `web` post tests prove present permalink href and
    visible timestamp output, absent-permalink non-link output, and the
    generated edit href remain unchanged.

## Risk checks

- The two tasks own disjoint production seams: Task 1 owns `web/src/sidebar/**`;
  Task 2 owns the relevant `web/src/posts/{render,component}.rs` sites. Shared
  test-support changes require coordination before editing.
- Static sidebar URL validation executes once, not once per reactive render.
- No empty string represents a missing post permalink after Task 2.
- Every Leptos URL conversion follows the existing final-view-site pattern
  because `RootRelativeUrl` does not implement `IntoAttributeValue`.
- All `PostView` construction sites and focused fixtures migrate together; no
  primitive compatibility path remains.
- Integrated verification runs the repository's changed-contract checks and
  normal commit gate after both slices land.
