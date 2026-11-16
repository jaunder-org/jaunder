# Issue #1936: back up now, and back up on shutdown

## Outcome

An operator can press **Back Up Now** on `/admin/backups` and get a backup
written immediately, and every graceful stop of `jaunder serve` (SIGTERM,
SIGINT, `jaunder shut-down`, `systemctl stop`/restart, `nixos-rebuild switch`)
writes one final backup before the process exits. Both produce exactly the
artifact the scheduled backup produces today.

## Approach and tradeoffs

Today `server/src/backup.rs` runs one job on the configured cron schedule:
`storage::export_backup` into a timestamped directory or `.tar.gz` under the
configured destination, then retention pruning, metrics and swallowed-error
reporting. The worker captures the whole `BackupConfig` once at startup.

This change turns that job into one **backup runner** reachable from three
triggers:

| Trigger   | When                                                 | On failure                                      |
| --------- | ---------------------------------------------------- | ----------------------------------------------- |
| Scheduled | cron fires (unchanged timing)                        | reported, swallowed (unchanged)                 |
| Manual    | operator presses **Back Up Now**                     | error shown on the page; detail reported        |
| Shutdown  | graceful drain, after HTTP, uploads and workers stop | reported, swallowed; the process exits normally |

- **One runner instance** is built in `serve` setup and shared by the router
  context (web reaches it through a trait it defines and the server provides via
  Leptos context, as `SiteIdentityPublisher` does under ADR-0016), the scheduled
  worker, and the drain in `cmd_serve`; the server test harness provides it too.
- **Every run reads the saved settings** (destination, retention, mode) when it
  starts. Only the cron expression, and whether the scheduled worker is enabled
  at all, stay fixed at startup; the page says so. Live schedule reload is a
  follow-up issue.
- **Run-at-a-time guard** held across export _and_ prune. The export already
  serializes on the database lock, but prune and the manual refusal need the
  runner's own guard: Manual tries to acquire and is refused with "a backup is
  already running" if held; Scheduled and Shutdown wait for it.
- **Same-second names.** Backup names have one-second resolution and export
  refuses an existing path. Because runs are serialized, the runner simply waits
  for the next second when the current timestamped name is taken (at most one
  second). Naming format is unchanged.
- **Archive written atomically.** Archive mode currently writes straight to the
  final `backup-*.tar.gz`, so a killed run leaves a truncated archive that
  pruning counts as a backup. It will write to a temporary sibling name pruning
  ignores and rename into place on success. A killed directory backup already
  lacks `manifest.json` and is never counted (leftover cleanup stays out of
  scope).
- **Manual trigger** is a new operator-only server function. The runner job is
  spawned as its own task and the request awaits its result, so a client
  disconnect cannot abandon a half-written backup. The page shows the written
  path; the button is disabled while pending and when no destination is saved.
  Error mapping: already running → `Conflict`; no saved destination →
  `Validation`; export or prune failure → internal (generic text on the page per
  ADR-0017, detail in the error report). Rejected: start-and-poll status (needs
  persisted job state for no current requirement); browser download (a different
  feature).
- **Shutdown trigger** runs in `cmd_serve` after the background workers have
  drained and **before the signal supervisor is aborted**, so a second SIGINT
  still forces exit during a long backup, and before the storage lock is
  released, so no second server can start on this storage meanwhile. It runs
  whenever a destination is saved; no change detection. Accepted cost: every
  restart takes a full backup, lengthening stops and pushing older backups out
  of retention sooner.
- **Stop timeouts.** The NixOS module sets `TimeoutStopSec = 15min` on
  `jaunder.service` (systemd's 90 s default would SIGKILL a large backup).
  `jaunder shut-down --timeout` defaults to 900 s to match, amending ADR-0181's
  30 s default, which would otherwise routinely report failure on a stop that
  succeeds.
- **Diagnostics name the trigger**: trigger-specific swallowed-error contexts
  (`server.backup.scheduled_run`, `server.backup.shutdown_run`,
  `server.backup.manual_run`) and a neutral "backup complete" log with a trigger
  field. Metrics are unchanged.

New machinery: the guard, the runner trait/context wiring, the server function,
and the archive temp-then-rename. Export, pruning rule, metrics and restore are
reused unchanged.

## Acceptance

Proof to execute:

- Dual-backend server tests (`#[apply(backends)]`): an operator's manual backup
  writes a manifest-bearing backup under the saved destination and prunes to the
  retention count; a non-operator is refused; no saved destination returns the
  validation error; a manual request while the test holds the runner guard
  returns the conflict error.
- Runner unit tests: two back-to-back runs within one second both succeed with
  distinct names; a run reads settings saved after startup; an archive run
  interrupted before completion leaves nothing pruning counts.
- `cmd_serve` host tests: a graceful stop with a saved destination leaves
  exactly one new backup; with an unwritable destination the stop still
  completes `Ok`.
- E2E on `/admin/backups`: with a destination under the E2E server's writable
  storage directory saved, **Back Up Now** shows the written path; with none
  saved, the button is disabled; the original destination is restored after.
- Nix evaluation check that `jaunder.service` sets `TimeoutStopSec`.
- `visual-proof` before/after of `/admin/backups` at desktop and mobile widths,
  with and without a saved destination.

Deliverables to commit: code and tests above; the `docs/flows/administration.md`
census row and regenerated `docs/coverage/server-fns.json`; `jaunder shut-down`
help text; an ADR draft (via `jaunder-adr`) recording "graceful shutdown takes a
final backup", the stop-timeout consequences and the ADR-0181 amendment,
projected into `docs/ARCHITECTURE.md`; the follow-up issue for live schedule
reload.

## Boundaries

- No browser download, job queue, status polling, or change detection.
- No change to backup contents, naming format, pruning rule, or restore.
- No live reload of the cron schedule (follow-up issue).
- No backup on forced exit (second SIGINT, SIGKILL, crash).
- No cleanup of pre-existing partial directory backups.
- `jaunder backup` CLI unchanged.
