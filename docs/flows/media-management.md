# Media management

Matrix: `matrix:docs/coverage/csr-e2e-matrix.md#media-management`

## Routes

- `route:/media`
- `route:/app`
- `route:/posts/new`
- `route:/posts/:post_id/edit`

## Endpoint census

| Endpoint                        | Status  | Surface                                                                                                  |
| ------------------------------- | ------- | -------------------------------------------------------------------------------------------------------- |
| `endpoint:/api/media/list_mine` | Covered | Fills the `/media` library table with the authenticated user's uploaded files.                           |
| `endpoint:/api/media/get_usage` | Covered | Reports quota, used bytes, and max-file-size limits for the storage panel.                               |
| `endpoint:/api/media/upload`    | Covered | Handles the shared upload widget used on `/media`, the cockpit composer, and the full-page post editors. |
| `endpoint:/api/media/delete`    | Covered | Deletes media from the library, or refuses with referencing post ids until the user force-deletes.       |

`/media` is the inventory and cleanup surface: upload a file, inspect current
storage usage, and review every owned upload in one table. Successful uploads
bump both the usage panel and the list resource so the page re-reads
authoritative state.

The same upload widget is embedded in the cockpit composer and the full-page
create/edit post routes. Those routes own post mutation behavior, but media
upload itself stays one flow because every caller gets the same stored URL
result and the same multipart validation path.

Delete is intentionally two-stage. A normal delete asks storage for an atomic
decision; when the item is still referenced, the page renders the blocking post
ids and offers an explicit force-delete submission rather than silently
orphaning embeds.
