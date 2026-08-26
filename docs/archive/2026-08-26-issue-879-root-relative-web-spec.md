# Issue #879 — Retain RootRelativeUrl through sidebar and post links

## Outcome

Root-relative sidebar and post links retain `RootRelativeUrl` from their
construction boundary until Maud or Leptos requires an attribute value. Existing
navigation, post rendering, and route behavior remain unchanged.

The issue's stale classification is corrected: sidebar icon paths are SVG path
data, and post banners are status text. Neither is modeled as a URL.

## Load-bearing decisions

- ADR-0063 §4–§5 governs the change: parse at the outermost boundary, keep the
  existing domain type inward, and flatten only at an external framework
  consumer.
- The shared sidebar navigation catalog stores typed `RootRelativeUrl` values.
  Its static paths are initialized and validated once rather than reparsed on
  each reactive render.
- Operator-only sidebar destinations follow the same typed-static pattern as the
  shared catalog.
- `SidebarNavItem` borrows an optional typed URL. Leptos receives an
  attribute-compatible string only where the anchor is rendered.
- `PostView.permalink` is `Option<&RootRelativeUrl>`. A missing permalink
  remains `None`; the implementation does not manufacture an empty-string
  sentinel.
- `PostCard` constructs its `/posts/{post_id}/edit` route as `RootRelativeUrl`
  and retains that type until the anchor attribute boundary.
- Known-valid generated routes use the repository's existing
  parse-and-unreachable pattern. Validation rules remain owned by
  `RootRelativeUrl`; no trusted constructor is added.
- `RootRelativeUrl` remains an owned, runtime-capable string newtype. No const
  constructor or representation redesign is introduced.
- `SidebarNavItem::icon_path` remains SVG `d`-attribute data. `PostView::banner`
  remains optional status/display text.

## Acceptance

- `SidebarNavItem::href` is typed as an optional borrowed `RootRelativeUrl`.
- Every URL in the shared navigation catalog and both operator-only sidebar
  destinations is stored as `RootRelativeUrl` and initialized once.
- Anonymous Maud sidebar output and authenticated Leptos sidebar behavior remain
  equivalent to their current behavior, including active classes and non-link
  placeholder items.
- `PostView.permalink` is `Option<&RootRelativeUrl>` at every construction site
  and render helper.
- Rendering a post without a permalink preserves absence without an empty URL
  sentinel and retains the current non-link behavior.
- Rendering a post with a permalink produces the same href and visible timestamp
  as before.
- `PostCard`'s generated edit destination is a `RootRelativeUrl` before it
  reaches Leptos's `href` attribute.
- Regression coverage exercises sidebar links, present and absent post
  permalinks, and the generated edit link through observable rendered output or
  component behavior.
- Existing `RootRelativeUrl` malformed-input rejection is unchanged.

## Boundaries

- No new SVG-path, banner-text, label, timestamp, route-segment, or absolute-URL
  domain type.
- No repo-wide URL adoption sweep beyond the sidebar catalog, `SidebarNavItem`,
  `PostView`, and `PostCard` edit destination named above.
- No changes to public route shapes, redirects, active-route selection,
  authorization, post visibility, or rendering copy.
- No changes to `RootRelativeUrl` validation, serialization, or storage
  representation.
- Framework-required string conversion at the final Maud/Leptos attribute site
  is intentional and remains permitted.
