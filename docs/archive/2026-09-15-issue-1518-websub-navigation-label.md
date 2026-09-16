# Issue #1518: WebSub navigation label

## Outcome

An operator sees **WebSub** in the authenticated sidebar for the `/admin/websub`
destination. The navigation label matches the destination page title instead of
presenting the page as only a recovery surface.

## Load-bearing decisions

- The sidebar label is exactly **WebSub**.
- The existing `/admin/websub` destination, active-item identity, icon,
  ordering, and operator-only visibility remain unchanged.
- The destination remains the complete operator WebSub surface: hub
  configuration plus regeneration and publication dead-letter recovery.
- This is a navigation-copy correction, not a rename of the WebSub domain or its
  recovery operations.
- No architectural decision or domain-language change is introduced.

## Acceptance

- The authenticated operator sidebar renders a **WebSub** link to
  `/admin/websub`.
- The sidebar does not render **WebSub Recovery** as that destination's label.
- Anonymous and authenticated non-operator viewers still cannot see the
  operator-only destination.
- Automated regression coverage proves the exact label and unchanged route.
- The repository's focused verification for the changed sidebar behavior passes.

## Boundaries

- Do not change the `/admin/websub` page title, forms, tables, explanatory copy,
  or recovery behavior; issue #1519 owns page explanation improvements.
- Do not change WebSub storage, server functions, CLI commands, routes, or
  authorization.
- Do not rename other administration navigation items or undertake broader
  sidebar copy normalization.
