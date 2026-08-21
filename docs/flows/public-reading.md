# Public reading

Matrix: `matrix:docs/coverage/csr-e2e-matrix.md#public-reading`

## Routes

- `route:/`
- `route:/:username`
- `route:/~:username/:year/:month/:day/:slug`

## Endpoint census

| Endpoint                                     | Status  | Surface                                                                                                              |
| -------------------------------------------- | ------- | -------------------------------------------------------------------------------------------------------------------- |
| `endpoint:/api/timeline/list_local_timeline` | Covered | Feeds the site-wide `/` timeline, including load-more and same-route refresh after owner-side mutations.             |
| `endpoint:/api/timeline/list_by_user`        | Covered | Feeds the mounted user timeline matcher; rendered links stay canonical with the `~username` form.                    |
| `endpoint:/api/posts/get`                    | Covered | Resolves the permalink page and upgrades it for the author when the same URL names a private or draft post they own. |

`/` is always the enhanced public local timeline, even for the signed-in owner.
The projector seed is adopted for first paint, then the CSR timeline keeps
paging and refresh in place without swapping the route to `/app`.

The mounted user matcher renders a public profile timeline with canonical
`~username` links, feed discovery, and an optional subscription control for
eligible viewers. The read path stays visibility-filtered: a viewer only gets
posts they are allowed to see.

The permalink page reuses the server-painted post body as its first-paint
fallback, then re-fetches so the author can regain edit, delete, publish, or
unpublish affordances on the same URL. Outsiders still get an indistinguishable
not-found when the permalink only resolves to a private or draft post.
