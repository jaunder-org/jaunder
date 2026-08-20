# Tag browsing

Matrix: `matrix:docs/coverage/csr-e2e-matrix.md#tag-browsing`

## Routes

- `route:/tags/:tag`
- `route:/:username/tags/:tag`
- `route:/:username`
- `route:/~:username/:year/:month/:day/:slug`

## Endpoint census

| Endpoint                                      | Status  | Surface                                                                     |
| --------------------------------------------- | ------- | --------------------------------------------------------------------------- |
| `endpoint:/api/timeline/list_by_tag`          | Covered | Fills the site-wide tag page at `/tags/:tag`.                               |
| `endpoint:/api/timeline/list_by_user_and_tag` | Covered | Fills the per-author tag page reached from canonical `~username` tag links. |

Tag browsing is entirely read-side: the user clicks a tag chip from a public
timeline or permalink and lands on either the site-wide tag page or the
per-author variant. Both routes parse the tag client-side into its canonical
lowercase form before any request, so the heading, fetch, and projected seed
stay in sync.

The two tag pages reuse the shared timeline gate, load-more behavior, and
empty-state copy, but they differ in scope. The site-wide page queries every
visible post carrying the tag, while the per-author page intersects the same tag
with one author's namespace.

Author-side tag autocomplete is deliberately out of scope here even though it
uses the same domain vocabulary. This document owns the read-side tag timelines
only; the write-side suggestion endpoint is assigned to the authenticated
cockpit's inline composer.
