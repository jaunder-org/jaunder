# ADR-DRAFT: Keep dev runs at worktree removal

- Status: proposed
- Date: 2026-11-26
- Issue: [#2138](https://github.com/jaunder-org/jaunder/issues/2138)

## Context

`dev` traces and `traced-run` output live in a checkout's `.xtask/run/`, so they
disappear with the worktree. The milestone 33 retrospective (#1859) found 10 of
77 recent worktrees still on disk, 357 of 2,872 `traced-run` outputs left, and
many hook and landing outcomes unrecoverable. Its evidence came from agent
transcripts instead. The documented remedy, copying a run by hand before
removal, was not followed.

Worktrees are removed with Worktrunk's `wt remove`, from unmanaged cleanup after
`dev pr-cleanup` (which detaches and deletes the branch but leaves the
worktree), from Herdsman's managed retirement, and ad hoc. Only the removal
itself is common to all of them.

## Decision

- A Worktrunk `pre-remove` project hook (`.config/wt.toml`) runs
  `direnv exec . dev keep-runs || true` in the worktree being removed, using
  that worktree's own devShell.
- `dev keep-runs` copies each `.xtask/run/<run_id>.trace.d/`,
  `<run_id>.trace.json` and `traced-run` summary `<id>.run.json` into
  `${XDG_STATE_HOME:-~/.local/state}/jaunder/runs/<checkout directory name>/`.
  It never copies the parked stdout/stderr bodies. The key is the directory
  name, since the branch may already be deleted.
- The store is capped at 20 GiB. After each copy, whole per-checkout directories
  are deleted, least recently kept first, sparing the one just written.
- A failed copy warns and exits 1, but the hook never blocks removal.
- `traced-run` parks its JSON summary as `<id>.run.json`, beside the output.
- The kept spans carry what a retrospective needs: the root's
  `vcs.ref.head.name`/`vcs.ref.head.revision`, and each failed gate's
  `jaunder.failed_step`/`jaunder.failed_reason` (its first unsuccessful span).
  The precommit gate adds its route and restage count, and pr-watch adds its
  final report's fields. A report kept as span attributes needs no extra file.

## Consequences

- Evidence for a removed worktree survives on that machine without anyone
  remembering a copy. This is a bounded copy at removal, not a scheduler or a
  cross-machine collector, and CI artifacts keep their own 14-day retention.
- Worktrunk asks once per machine to approve the hook. Until then,
  non-interactive removal or `--no-hooks` skips it, and a worktree whose base
  predates the hook has none.
- A failure loses that worktree's evidence, but only with a visible warning.
- Only the checkout root's `.xtask/run/` is kept, and the parked output bodies
  go with the worktree, so the summaries and spans have to be enough.
- Cargo and kache cache counts remain unrecorded.
