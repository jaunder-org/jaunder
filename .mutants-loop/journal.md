# Journal

Append-only. Newest last. One line per event.

- 2026-08-06 — groundwork laid: worktree, protocol, queue, discovery script.
  2339 candidate mutants across 9 packages. Discovery not yet started.
- 2026-08-06 — branch fast-forwarded to main (525420d4). Groundwork committed
  (d23d0ca4). Discovery restarted from the new base.
- 2026-08-06 — killed common/src/backup.rs:48 `BackupMode::label -> "xyzzy"`.
  The existing test asserted only `!label().is_empty()`; pinned the authored
  labels instead. backup.rs now 0 missed / 3 caught.
