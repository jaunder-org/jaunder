# Issue #1439: Compact draft Post actions

## Outcome

The authenticated unpublished-Post queues present row actions without making the
action controls dominate each row's height. Draft actions remain visible,
discoverable, and usable at ordinary and narrow viewport widths.

## Load-bearing decisions

- The existing action group becomes a horizontal row that wraps when space is
  constrained; it does not become an Actions disclosure or icon-only control.
- Each action keeps its full text label. Draft actions remain ordered Edit,
  Publish, Delete.
- Edit and Publish remain neutral `j-btn` actions. Delete remains
  `j-btn is-danger` and retains its confirmation.
- The shared action layout also governs Scheduled Post rows, without adding,
  removing, or changing their current actions.
- The Post title, slug, scheduling badge, and Permalink remain in the content
  portion of the row.
- Native links and forms, mutation behavior, authorization, focus behavior, and
  accessible names do not change.

## Acceptance

- A draft row containing Edit, Publish, and Delete no longer derives its minimum
  height from a three-button vertical stack.
- Under the existing Desktop Chrome visual project, the three draft actions
  appear as one compact horizontal group aligned within the row.
- At the established 375×800 narrow viewport, actions wrap within their group
  without clipping, overlapping Post content, overflowing the viewport, or
  losing full labels.
- Scheduled Post rows retain their single Edit action and align through the same
  action-group layout.
- Publishing and deleting still settle through the existing forms, Delete still
  asks for confirmation, and the visible action order and styling hierarchy are
  unchanged.
- Row-scoped visual coverage constrains the affected content/action geometry at
  both viewport fixtures without pinning unrelated page chrome.

## Boundaries

- This issue does not change Post lifecycle semantics, action availability,
  mutation endpoints, authorization, routing, pagination, or result messages.
- It does not change full-versus-compact composer geometry owned by #1437 or
  site-wide form presentation owned by #1438.
- It does not reuse or alter the trusted Post-header Actions disclosure, create
  a new layout primitive, change the public Style Contract, or require an ADR or
  domain-glossary update.
