# PR observer

Host-only, read-only GitHub PR evidence observation. From the checkout's
devShell:

```sh
just pr-watch 731
just pr-watch 731 --once
just pr-watch 731 --until merged
```

Omit the number to resolve the open PR for the current branch. The default polls
at 30-second intervals for up to 90 minutes and stops at the next
caller-actionable outcome. `--once` reports one observation; `--until merged`
passively continues across readiness while another actor may arm the merge.
Neither mode authorizes or performs landing, queue re-enqueue, cleanup, or
tracker changes.

Stdout is one JSON observer report (not an xtask command envelope); stderr
carries progress and diagnostics. `outcome` is authoritative:

- `ready-to-land` and `merged` exit 0. Readiness requires nonempty live-ruleset
  requirements, complete current-head evidence, and a landable GitHub state.
  Readiness is **not** approval to merge.
- `checks-failed` means a directly or transitively required check failed;
  `watcher-error` means observation could not establish trustworthy evidence.
- `timed-out` means the observation budget expired; `pending` is a one-shot
  observation whose evidence or checks are still incomplete.
- `blocked`, `conflicted`, `stale`, and `closed-unmerged` preserve GitHub's
  adverse state. `dequeued` means an observed same-head queue entry vanished;
  `ejected` identifies a failed current-head merge-group run.

All adverse/pending outcomes exit 1; invalid arguments or failure to establish a
subject exit 2. The report includes the exact head and available log pointers.
The tool reads required contexts from the live main ruleset, never a local job
allowlist. Empty/incomplete evidence cannot establish success.

During migration, xtask's existing PR frontdoor uses this package's same polling
and evidence classifiers. The observer accepts no arming or cleanup capability;
mutation-only transport helpers and the privileged ADR promoter remain in xtask.
