# Administration

Matrix: `matrix:docs/coverage/csr-e2e-matrix.md#administration`

## Routes

- `route:/admin/site`
- `route:/admin/backups`

## Endpoint census

| Endpoint                                         | Status  | Surface                                                                                            |
| ------------------------------------------------ | ------- | -------------------------------------------------------------------------------------------------- |
| `endpoint:/api/site/get_identity`                | Covered | Seeds the site title and base-URL form on the site-settings page.                                  |
| `endpoint:/api/site/update_identity`             | Covered | Persists the operator's title/base-URL changes from the typed settings form.                       |
| `endpoint:/api/site/is_base_url_warning_visible` | Covered | Drives the soft shell warning when feeds and AtomPub are disabled by a missing base URL.           |
| `endpoint:/api/backup/is_warning_visible`        | Covered | Drives the soft shell warning when scheduled backups still lack a destination.                     |
| `endpoint:/api/backup/get_settings`              | Covered | Seeds the backup configuration form with the persisted schedule, destination, retention, and mode. |
| `endpoint:/api/backup/update_settings`           | Covered | Persists typed backup settings updates from the backups page.                                      |

Administration is split into two direct-entry operator routes: site identity and
backups. Both pages load the persisted settings first, then submit typed
direct-bind updates so blank optional fields clear configuration while malformed
values fail before dispatch.

The warning endpoints belong here even though their banners render in shared
authenticated chrome. They are soft checks, not authorization challenges:
non-operators and stale cookie-only sessions simply hide the banners, while
operators get links into the exact admin page that resolves the warning.

The routes themselves stay narrow. `/admin/site` owns site title and canonical
base URL, while `/admin/backups` owns storage destination, schedule, retention,
and backup mode. Execution of backup jobs and protocol behavior that depends on
the saved base URL live outside this CSR flow.
