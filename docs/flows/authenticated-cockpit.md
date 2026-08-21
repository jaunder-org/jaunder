# Authenticated cockpit

Matrix: `matrix:docs/coverage/csr-e2e-matrix.md#authenticated-cockpit`

## Routes

- `route:/app`

## Endpoint census

| Endpoint                                | Status  | Surface                                                                                                   |
| --------------------------------------- | ------- | --------------------------------------------------------------------------------------------------------- |
| `endpoint:/api/timeline/list_home_feed` | Covered | Loads the signed-in author's personalized home feed after the shared session reconcile confirms identity. |
| `endpoint:/api/tags/list`               | Covered | Powers the debounced tag autocomplete inside the inline composer that lives directly on `/app`.           |

`/app` is the directly-bookmarkable authenticated feed. It does not trust the
advisory local marker by itself: the page waits for the shell's shared session
reconcile, bounces anonymous or expired visitors to `/login`, and only then
fetches the viewer's own published-post timeline.

Once the reconcile resolves, the route keeps one screenful of chrome alive:
topbar, inline composer, and the feed rows. Publishing or saving from the
compact composer bumps the feed resource in place instead of remounting the
page.

This doc owns the cockpit-only read path and its inline tag suggestions. Session
reconciliation itself stays with the shell flow, while post creation and media
upload behavior are covered by the authoring and media documents.
