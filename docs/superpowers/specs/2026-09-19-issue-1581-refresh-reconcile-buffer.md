# Refresh the Jaunder reconciliation report

## Outcome

Pressing `g` in a `jaunder-reconcile` report refreshes the report from current
local and remote state. Pulling selected Posts remains an explicit operation and
moves to `f`, for “fetch selected.”

## Load-bearing decisions

- `g` means refresh, following the conventional Emacs report-buffer interaction.
- `f` invokes the existing pull-selected operation; its safety, confirmation,
  eligibility, and batch semantics do not change.
- A manual refresh rebuilds the complete reconciliation inventory rather than
  repainting stale report data.
- Refresh restores point to the same stable row and column, clamped to that
  row's new line end; if the row disappeared, point moves to the buffer start.
- Refresh retains marks only for stable rows still present and discards marks
  for absent rows so they cannot silently reappear later.
- Refresh preserves the ordered **Last batch** summary.
- A failure before a complete replacement report exists re-signals the original
  Emacs error and leaves the rendered text, report object, point, marks, and
  **Last batch** unchanged.
- Row-local Member failures remain report data: a completed inventory refresh
  displays them as refreshed `unclassifiable` rows rather than failing the whole
  refresh.

## Acceptance

- In a reconciliation report, `g` obtains fresh inventory and displays the new
  classification.
- In that report, `f` runs pull-selected and `g` no longer does so.
- A successful refresh restores and clamps point when its row remains, falls
  back to buffer start otherwise, prunes vanished marks, retains surviving
  marks, and retains the ordered **Last batch** summary.
- A failed refresh re-signals its original error without changing any displayed
  or buffer-local report state.
- Distinct ERT assertions cover exact `g`/`f` bindings, changed-inventory
  reclassification, point restoration/clamping/fallback, mark retention and
  pruning, ordered **Last batch** retention, row-local failure rendering, and
  atomic failed-refresh behavior.
- Protocol Client documentation describes `g` as refresh and `f` as fetch.

## Boundaries

- Push, pull, remote-delete, selection, confirmation, and batch execution
  semantics are otherwise unchanged.
- This does not add automatic or periodic synchronization.
- This does not change AtomPub, server, storage, or web behavior.
- No new architectural decision or domain vocabulary is introduced.
