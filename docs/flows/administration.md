# Administration

Matrix: `matrix:docs/coverage/csr-e2e-matrix.md#administration`

## Routes

- `route:/admin/site`
- `route:/admin/backups`
- `route:/admin/websub`
- `route:/admin/smtp`

## Endpoint census

| Endpoint                                          | Status  | Surface                                                                                            |
| ------------------------------------------------- | ------- | -------------------------------------------------------------------------------------------------- |
| `endpoint:/api/site/get_identity`                 | Covered | Seeds the site title and base-URL form on the site-settings page.                                  |
| `endpoint:/api/site/update_identity`              | Covered | Persists the operator's title/base-URL changes from the typed settings form.                       |
| `endpoint:/api/site/get_media_uploads_enabled`    | Covered | Independently loads the site-wide Media Upload Capability for its dedicated admin card.            |
| `endpoint:/api/site/update_media_uploads_enabled` | Covered | Independently persists the Media Upload Capability without writing site identity settings.         |
| `endpoint:/api/site/is_base_url_warning_visible`  | Covered | Drives the soft shell warning when feeds and AtomPub are disabled by a missing base URL.           |
| `endpoint:/api/backup/is_warning_visible`         | Covered | Drives the soft shell warning when scheduled backups still lack a destination.                     |
| `endpoint:/api/backup/get_settings`               | Covered | Seeds the backup configuration form with the persisted schedule, destination, retention, and mode. |
| `endpoint:/api/backup/update_settings`            | Covered | Persists typed backup settings updates from the backups page.                                      |
| `endpoint:/api/smtp/get_settings`                 | Covered | Seeds the secret-free SMTP relay form and reports only whether a password exists.                  |
| `endpoint:/api/smtp/update_settings`              | Covered | Atomically persists operator-managed relay settings and paired credential intent.                  |
| `endpoint:/api/websub/get_websub_settings`        | Covered | Seeds the configured hub form from the coherent publisher snapshot.                                |
| `endpoint:/api/websub/update_websub_hub`          | Covered | Applies an operator-authorized, generation-fenced hub mutation.                                    |
| `endpoint:/api/websub/list_dead_letters`          | Covered | Pages regeneration or publication dead letters for operator recovery.                              |
| `endpoint:/api/websub/redrive_dead_letters`       | Covered | Atomically returns the exact selected dead-letter IDs to the eligible phase.                       |

Administration is split into four direct-entry operator routes: site identity,
backups, SMTP relay configuration, and `WebSub` recovery. Each page loads
persisted state first, then submits typed updates. Blank optional settings clear
configuration while malformed values fail before dispatch; SMTP passwords are
write-only and persisted SMTP changes require an external restart.

The warning endpoints belong here even though their banners render in shared
authenticated chrome. They are soft checks, not authorization challenges:
non-operators and stale cookie-only sessions simply hide the banners, while
operators get links into the exact admin page that resolves the warning.

The routes themselves stay narrow. `/admin/site` owns site title, canonical base
URL, and the independently persisted Media Upload Capability; `/admin/backups`
owns storage destination, schedule, retention, and backup mode; `/admin/smtp`
owns persisted outbound relay and paired credential intent; and `/admin/websub`
owns the publisher hub plus regeneration and publication dead-letter recovery.
Runtime mailer reload, execution of backup jobs, and publisher delivery live
outside this CSR flow.
