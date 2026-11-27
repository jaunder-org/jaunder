# Keep dev traces and run summaries after worktree removal (#2138)

## Outcome

When `wt remove` deletes a jaunder worktree, the worktree's `dev` traces and
`traced-run` summaries are first copied into a per-user store capped at 20 GiB.
A later retrospective can then read how long hooks and gates took, whether they
failed, and on which step, with `dev trace summary`, without reconstructing it
from agent transcripts.

## Approach and tradeoffs

**When the copy happens.** The project adds a Worktrunk `pre-remove` hook in
`.config/wt.toml`: `direnv exec . dev keep-runs || true`. Worktrunk runs it
inside the worktree being removed, before deleting it. `direnv exec .` takes
`dev` from that worktree's own devShell, so a hook never depends on the PATH of
whoever ran `wt` or a stale direnv. Worktrunk reads the project config from the
worktree it runs in, so the hook fires once this change is on that worktree's
base. That covers every removal: after `dev pr-cleanup` (which only detaches and
deletes the branch, so the worktree is still there afterwards), Herdsman's
managed removals, and ad-hoc ones. The simpler alternative, a copy step inside
`scripts/pr-cleanup.sh`, would miss managed work (which never runs pr-cleanup)
and any worktree removed without it. Cost: Worktrunk asks once per machine to
approve the project hook. A non-interactive removal before that approval skips
it, as does `wt remove --no-hooks`, so CONTRIBUTING documents the one-time
`wt config approvals add`.

**Failure never blocks removal** (human decision). `dev keep-runs` warns on
stderr and exits 1 if a copy fails. The hook's `|| true` lets the removal go
ahead, including when `dev` is not on PATH. Evidence can therefore be lost, but
only with a visible warning.

**What `dev keep-runs` does.** It runs in the checkout and copies from
`.xtask/run/` to
`${XDG_STATE_HOME:-~/.local/state}/jaunder/runs/<checkout directory name>/`. It
copies every `<run_id>.trace.d/` (the span records `dev trace summary` reads),
every `<run_id>.trace.json` (the Perfetto view) and every `traced-run` summary
`<id>.run.json`. It never copies the parked `.out`/`.err` bodies. The store is
keyed by directory name rather than branch because pr-cleanup has already
detached and deleted the branch by the time removal runs. Re-running the command
overwrites the same names, so a retry is harmless. A checkout with no runs
copies nothing and succeeds. The command can also be run by hand before a
removal without Worktrunk.

**The cap.** After copying, it deletes whole per-worktree directories in the
store, oldest modification time first, until the store totals at most 20 GiB
(human decision). It never deletes the directory it has just written, so one
oversized worktree can leave the store above the cap until the next keep. No
scheduler and no cross-machine collection are involved: the store is a bounded
copy taken at removal, which the milestone's exclusion of a retention collector
does not cover (human decision).

**`traced-run` writes its summary to disk.** Besides printing it, traced-run
writes the same JSON to `.xtask/run/<id>.run.json`. It has no other way to
survive, since it is currently only on stdout. A failed write warns on stderr,
following traced-run's existing ancillary warnings, and leaves stdout and the
exit code unchanged.

**Richer spans.** The trace records gain these attributes (OTel's `vcs.*` names
where they exist):

| Span                 | Attribute                                                                                 | Value                                                                                                                    |
| -------------------- | ----------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| root                 | `vcs.ref.head.name`, `vcs.ref.head.revision`                                              | branch (left out when detached), full HEAD SHA (in precommit this is the parent of the commit being made)                |
| each `dev` gate span | `jaunder.failed_step`, `jaunder.failed_reason`                                            | only when the gate does not end `ok`: the first span in that process to end `failed`/`signalled`, and its reason (below) |
| `precommit` gate     | `jaunder.precommit.route`, `jaunder.precommit.route_reason`, `jaunder.precommit.restaged` | `staged-markdown-only` or `broad`; the routing reason; the number of paths auto-staged                                   |
| `pr-watch` gate      | `jaunder.pr.number`, `.head_sha`, `.outcome`, `.pointer`, `.detail`                       | the final report's fields, `detail` capped at 1 KiB                                                                      |

For the reason, `StepFailed` gains an optional message. `refuse()`,
`index._fail` and pr-watch's refusals fill it in, and the reason is that
message's first line capped at 256 bytes. A child process's failure is
`exited <code>` or `signalled <n>`.

The hook kind needs no new attribute: the root span's name is already
`precommit` or `prepush`. `pr-land`'s final report is pr-watch's, which runs as
its child, so pr-land's trace carries it through the last pr-watch span (an
earlier one is the preflight). One limit remains: when `pr-land.sh` refuses on
its own, before or after pr-watch, its reason is only `exited <code>`, because
the script's stderr is not recorded. Keeping the report as span attributes means
the kept records already hold it, with no extra file. As now, a missing value is
left out and tracing never changes a step's outcome.

An ADR draft (`jaunder-adr`) records the store and the removal hook. It replaces
CONTRIBUTING's current "copy it to a destination you choose" advice.

## Acceptance

Proof to execute:

- pytest: `keep-runs` copies `.trace.d/`, `.trace.json` and `.run.json` but not
  `.out`/`.err`; a checkout with no runs succeeds and creates nothing; a copy
  failure warns and exits 1; pruning removes the oldest directories until the
  store is within the cap and spares the one just written. `dev trace summary`
  reads a kept `.trace.d/`.
- pytest: a failing prepush step (for example a refusal) names itself and its
  reason on the gate span. The precommit route and restage count, the root
  branch/HEAD and the pr-watch report attributes are recorded, and the
  span-record golden is updated.
- traced-run Rust tests: `<id>.run.json` equals the printed summary, and a
  failure to write it warns and leaves the summary and exit code unchanged.
- Manual: `wt remove` of a throwaway worktree, run from a shell without the
  devShell on PATH, leaves its runs in the store.

Deliverables to commit: `.config/wt.toml`, `dev keep-runs`, the traced-run
change, the span attributes, tests, the ADR draft, and two CONTRIBUTING
sections. "Observation and traces" gets the store path, the cap, the hook, its
failure behaviour and the approval step. The post-merge cleanup section says
that `wt remove` keeps runs and that `--no-hooks` skips the copy.

## Boundaries

- No Cargo/kache hit counts (the issue marks them optional). This is deferred.
- No change to `dev trace summary`'s output, to CI artifact retention, or to
  pr-cleanup.
- No copying of parked stdout/stderr, and no pruning of checkout-local
  `.xtask/run/`.
- Only the checkout root's `.xtask/run/` is kept. `traced-run` started from a
  subdirectory parks its output there, and that output is not kept.
