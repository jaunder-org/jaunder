# Theme management

Matrix: `matrix:docs/coverage/csr-e2e-matrix.md#theme-management-api-transport`

## Routes

No mounted CSR route exists yet. Theme management is an authenticated
server-function transport surface; Studio and administration UI ownership
remains a later task.

## Endpoint census

| Endpoint                                | Status        | Surface                                                                                              |
| --------------------------------------- | ------------- | ---------------------------------------------------------------------------------------------------- |
| `endpoint:/api/themes/list`             | API transport | Lists the authenticated author's or operator's owned theme catalog.                                  |
| `endpoint:/api/themes/get_draft`        | API transport | Reads an owner-authorized editable draft without exposing it to another catalog owner.               |
| `endpoint:/api/themes/get_presentation` | API transport | Reads owner-authorized logo/header bindings, pool members, and shuffle seed without Media owner IDs. |
| `endpoint:/api/themes/get_selection`    | API transport | Reads the site selection or the author's explicit override/inheritance state.                        |
| `endpoint:/api/themes/create`           | API transport | Creates a validated custom-theme draft in the selected owner catalog.                                |
| `endpoint:/api/themes/import_css`       | API transport | Creates an unselected private version-1 no-asset draft from bounded plain CSS.                       |
| `endpoint:/api/themes/import_package`   | API transport | Replaces a draft from a complete validated package/editor payload.                                   |
| `endpoint:/api/themes/import_zip`       | API transport | Imports a bounded ZIP Theme Package as a new unselected private draft.                               |
| `endpoint:/api/themes/replace_css`      | API transport | Replaces one owned draft's stylesheet while preserving its manifest and package assets.              |
| `endpoint:/api/themes/export`           | API transport | Exports the exact owner-authorized draft package without relational Media bindings.                  |
| `endpoint:/api/themes/rename`           | API transport | Renames an owned catalog entry under case-insensitive catalog uniqueness rules.                      |
| `endpoint:/api/themes/remove`           | API transport | Removes an owned theme and releases its guarded presentation bindings atomically.                    |
| `endpoint:/api/themes/publish`          | API transport | Validates and materializes the complete draft as an immutable published revision.                    |
| `endpoint:/api/themes/select`           | API transport | Selects a built-in or owned custom theme, or clears an author override to inherit.                   |
| `endpoint:/api/themes/replace_binding`  | API transport | Replaces a fixed logo or header binding after owner and Media checks.                                |
| `endpoint:/api/themes/replace_pool`     | API transport | Replaces the explicit header image pool and its deterministic shuffle seed.                          |
| `endpoint:/api/themes/shuffle`          | API transport | Replaces only the persisted header-pool shuffle assignment.                                          |
| `endpoint:/api/themes/preview`          | API transport | Renders an owned draft through the Style Contract renderer without changing selection.               |
| `/themes/draft/{theme_id}/{path}`       | API transport | Serves an owned draft asset only to its author owner or an operator owning its site draft.           |

Task 8 provides API transport, authorization, and Playwright transport evidence
for this census. It does not claim CSR route or mounted-component coverage: no
theme-management page is mounted yet. Site-owned operations require an operator;
author-owned operations require the authenticated author, and every draft,
package, preview, asset, and binding read remains owner-scoped.
