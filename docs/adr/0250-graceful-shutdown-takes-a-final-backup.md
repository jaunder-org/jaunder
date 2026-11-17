# ADR-0250: Graceful shutdown takes a final backup

- Status: accepted
- Date: 2026-11-16
- Issue: [#1936](https://github.com/jaunder-org/jaunder/issues/1936)

## Context

Jaunder's only backup trigger was a cron-scheduled job that captured the whole
backup configuration once at startup. Data written between the last scheduled
run and a stop — a restart, an upgrade by `nixos-rebuild switch`, a
`jaunder shut-down` — had no backup until the next scheduled run, and an
operator who wanted a backup before a risky change had to stop the service and
run `jaunder backup` by hand.

Adding triggers raises three problems the single scheduled job never had. Two
runs can overlap: export already serializes on the database lock, but retention
pruning does not, and a manual request should be refused rather than queued
behind a run in progress. Backup names have one-second resolution and export
refuses an existing path, so two runs within one second would collide. And an
archive-mode backup was written straight to its final `backup-*.tar.gz` name, so
a run killed midway left a truncated archive that pruning counted as a backup.

A backup taken while stopping also lengthens the stop. systemd kills a unit
still stopping after its 90-second default, and the
[identity-verified local shutdown decision](0181-identity-verified-local-shutdown.md)
gave `jaunder shut-down` a 30-second default wait that reports failure on
timeout.

## Decision

**Every graceful stop of `jaunder serve` takes one final backup** whenever a
backup destination is saved, with no change detection. It runs in the serve
drain after HTTP, uploads and every background worker have stopped, while the
storage lock is still held (no second server can start on this storage
meanwhile) and before the signal supervisor is aborted (a second SIGINT still
forces exit during a long backup). Its failure is reported and swallowed; the
stop itself still succeeds. A forced exit, SIGKILL or crash takes no backup.

**One backup runner serves every trigger** — scheduled, the operator's manual
**Back Up Now**, and shutdown. A single runner instance built at the serve
composition root is shared by the scheduled worker, the router context (web
reaches it through a web-defined trait the server provides, per
[ADR-0016](0016-dependency-injection-and-appstate.md)) and the drain.

- A run-at-a-time guard is held across export _and_ prune. The manual trigger
  tries to acquire it and is refused with "a backup is already running";
  scheduled and shutdown runs wait for it.
- Every run reads the saved destination, retention and mode when it starts. Only
  the cron expression, and whether the scheduled worker runs at all, stay fixed
  at startup.
- Because runs are serialized, a timestamped name already taken means a run
  finished earlier in the same second; the runner waits for the next second
  rather than changing the naming format.
- The manual run executes on its own task, and the request awaits its result, so
  a client disconnect cannot abandon a half-written backup.

**An archive backup appears under its final name only by atomic rename.** Its
staged contents and the partly written archive live inside one work directory
beside the destination whose top level is neither a manifest-bearing directory
nor a `.tar.gz` file, so pruning and the last-success scan never count an
unfinished run.

**Stop timeouts make room for the backup.** The NixOS module sets
`TimeoutStopSec = 15min` on `jaunder.service`, and `jaunder shut-down --timeout`
defaults to 900 seconds to match. This amends ADR-0181's 30-second default,
which would otherwise routinely report failure on a stop that succeeds; its
non-escalation contract is unchanged.

Rejected: a manual start-and-poll status API (it needs persisted job state for
no current requirement), backing up only when data changed since the last backup
(change detection is a separate feature), and a browser download of the backup
(a different feature).

## Consequences

Every restart and upgrade writes a full backup, so stops take longer and older
backups leave the retention window sooner than the schedule alone implies.
Operators whose backups exceed fifteen minutes must raise `TimeoutStopSec` and
pass a larger `--timeout`; a stop that systemd kills mid-backup leaves only an
ignored work directory, never a counted partial archive.

Backup failures carry the trigger in their diagnostics:
`server.backup.scheduled_run` and `server.backup.shutdown_run` swallowed-error
contexts, the manual trigger's `server.backup.manual_run` error context (tagged
`backup.trigger=manual` at the web boundary), and a neutral "backup complete"
log with a `trigger` field. Backup metrics are shared by all triggers.

A live reload of the cron schedule is not part of this decision; the backups
page states that schedule changes, and enabling scheduled backups, take effect
after a restart. Leftover work directories and partial directory backups from
killed runs are not cleaned up.
