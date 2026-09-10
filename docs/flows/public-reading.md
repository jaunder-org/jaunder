# Public reading

Matrix: `matrix:docs/coverage/csr-e2e-matrix.md#public-reading`

## Routes

- `route:/`
- `route:/:username`
- `route:/tags/:tag`
- `route:/:username/tags/:tag`
- `route:/~:username/:year/:month/:day/:slug`

`/YYYY/MM/DD/slug` is intentionally absent from the mounted route declarations.
It is an inbound-only HTTP compatibility alias, not a CSR route or a link
target.

The four public timelines take one URL-only order state: a bare URL means Newest
and `?order=oldest` means Oldest. Unknown values fall back to Newest. The same
control is also present on the authenticated `/app` home feed; it is likewise
URL-only, so a bare `/app` never inherits a previous selection.

## Endpoint census

| Endpoint                                     | Status  | Surface                                                                                                                                     |
| -------------------------------------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `endpoint:/api/timeline/list_local_timeline` | Covered | Feeds the site-wide `/` timeline, including URL-selected Newest/Oldest order, load-more, and same-route refresh after owner-side mutations. |
| `endpoint:/api/timeline/list_by_user`        | Covered | Feeds the mounted user timeline matcher; it shares URL-selected order with the canonical `~username` links.                                 |
| `endpoint:/api/posts/get`                    | Covered | Resolves the permalink page and upgrades it for the author when the same URL names a private or draft post they own.                        |

The tag-browsing and authenticated-cockpit flow censuses own the other three
ordered timeline endpoints: `list_by_tag`, `list_by_user_and_tag`, and
`list_home_feed`.

## Canonical navigation and direct HTTP aliases

Jaunder renders and the CSR navigates only the canonical
`/~username/YYYY/MM/DD/slug` permalink. A direct `GET /YYYY/MM/DD/slug` is
resolved by the server before CSR boot: it searches anonymous-visible active
Posts across all Users and returns a same-origin `302` with
`Cache-Control: no-store` only when exactly one Post matches. That redirect
preserves the original query string unchanged. A malformed, zero-match, or
multiple-match request returns the existing no-store public SPA shell miss
instead. The
[WordPress-compatible permalink alias decision](../adr/0189-wordpress-compatible-permalink-alias.md),
recorded at `docs/adr/0189-wordpress-compatible-permalink-alias.md`, keeps the
compatibility route outside the CSR, preserving ADR-0076's `~`-only navigation
rule.

`/` is always the enhanced public local timeline, even for the signed-in owner.
The projector seeds the requested URL order for first paint; the CSR timeline
adopts that seed, keeps the selection through mount, and continues paging in the
same direction without swapping the route to `/app`.

The mounted user matcher renders a public profile timeline with canonical
`~username` links, feed discovery, and an optional subscription control for
eligible viewers. The site tag and per-user tag timelines use the same order
query contract. The read path stays visibility-filtered: a viewer only gets
posts they are allowed to see.

The permalink page reuses the server-painted post body as its first-paint
fallback, then re-fetches so the author can regain edit, delete, publish, or
unpublish affordances on the same URL. Outsiders still get an indistinguishable
not-found when the permalink only resolves to a private or draft post.
