# Explicit batch Post transfer from reconciliation

## Outcome

The Emacs Protocol Client lets a User select Posts in the Jaunder reconciliation
report and explicitly push them to, pull them from, or delete them from the
User's AtomPub Collection. Reconciliation remains an inventory and selection
surface; it never chooses a direction or mutates either side automatically.

A User can safely transfer a selection of at least 1,000 local Posts without one
failure discarding prior successes or a retry creating duplicate Posts.

## Load-bearing decisions

- The persistent reconciliation report is the sole batch-selection surface. It
  supports arbitrary marked rows and a contiguous selected region.
- Push, pull, and remote delete are distinct, explicit commands. Each previews
  its selected operation count and requires confirmation before the first
  mutation. Remote deletion has its own destructive confirmation and is never
  folded into push, pull, report refresh, or generic reconciliation.
- Push accepts local drafts and matched Posts classified as safely local-ahead.
  Pull accepts server-only Posts and matched Posts classified as safely
  server-ahead. Unchanged Posts are no-ops.
- Orphans, concurrent local-and-server edits, duplicate identities, filename or
  slug collisions, unclassifiable rows, and rows unsafe for the chosen direction
  are blocked with an explanation. Selection is not an overwrite or
  conflict-resolution escape hatch.
- Remote deletion accepts unambiguous `server-only`, `unchanged`, `local-ahead`,
  and `server-ahead` rows. The report fetches and displays each selected
  Member's fresh strong ETag before confirmation; deletion proceeds only while
  that ETag remains current. Concurrent-edit, duplicate-identity,
  unclassifiable, and local-only rows are blocked. Deletion means Jaunder's
  retained soft deletion, not physical erasure.
- After confirmed deletion of a matched Member, the corresponding local file is
  removed using the existing single-Post deletion semantics. A server-only
  deletion has no local file effect. No local removal occurs unless the server
  confirms deletion.
- Batch execution is deterministic and sequential. It reports visible progress,
  permits cancellation between Posts, continues after independent item failures,
  and ends with an aggregate per-Post result.
- Successful create, update, and pull operations record their server-confirmed
  identity, slug, ETag, synchronization time, and local-file effect before the
  next Post begins. A successful deletion records the reviewed identity, slug,
  and ETag, the confirmed `204`, and its local-file effect. Completed work
  survives cancellation or process interruption.
- Before a create request, the client durably records one stable create key and
  the request identity it represents. Retries reuse that key until the remote
  Post identity is recovered. Jaunder treats a used key as permanently consumed
  for that User: it replays the original active Post and never creates another,
  including after the former one-hour replay window; a deleted original remains
  consumed rather than authorizing replacement creation. The client removes the
  create intent only after it has durably recorded the returned Post ID.
- Pulling a matched `server-ahead` Post revalidates both snapshots from the
  report: the remote strong ETag and the local file identity and modification
  state. A modified visited buffer, changed file, changed remote Member, or
  destination collision blocks the pull. The client stages and verifies the
  complete replacement and its Media, then atomically replaces or renames the
  local file only after all prerequisites succeed. If the destination is open in
  a clean, unmodified buffer, the client refreshes that buffer to the installed
  bytes, keeps it unmodified, and preserves its window and point where possible.
- Each selected Post uses the existing single-Post mapping and safety policy:
  direct-root Org files, server-assigned identity and slug, conditional writes,
  referenced Media upload, durable Local Media Copies on pull, and no local
  destructive change before its remote mutation succeeds.
- The report is rebuilt from a fresh inventory after a batch finishes or is
  cancelled, so its classifications and available actions describe current local
  and remote state rather than the pre-operation snapshot.

## Acceptance

- From one reconciliation report, a User can mark arbitrary rows or select a
  contiguous range and invoke push, pull, or remote delete for that selection.
- Pushing a mixed selection creates eligible local drafts, updates eligible
  local-ahead Posts, skips unchanged Posts, and visibly blocks every unsafe row
  without mutating it.
- Pulling a mixed selection installs eligible server-only Posts and updates
  eligible server-ahead local Posts. A remote change, local file change,
  modified visited buffer, or filename collision after report generation blocks
  matched replacement without overwriting it. A clean visited destination is
  refreshed to the installed bytes without becoming modified or losing its
  window. Unchanged Posts are skipped and every unsafe row is visibly blocked.
- Remote deletion requires a separate destructive confirmation, rejects stale
  ETags without local deletion, removes a matched local file only after
  confirmed server deletion, and accurately describes retained soft deletion.
- A batch with successes and failures preserves and records each success,
  continues with independent eligible Posts, and presents actionable failures in
  its final refreshed report.
- Cancelling between items leaves completed items durable and untouched items
  unchanged. Re-running after cancellation or an indeterminate create response,
  even beyond one hour, does not duplicate a Post.
- Dual-backend storage and HTTP tests prove that a keyed create replays its
  active original as `200` beyond one hour and after maintenance cleanup, that a
  deleted original returns `409` while keeping the key consumed, and that
  neither path increases the Post count.
- Pure ERT covers selection, direction eligibility, blocked states, progress,
  cancellation, aggregation, retry state, and matched-pull revalidation. A
  synthetic batch executes at least 1,000 eligible items through the executor
  and proves stable order, exactly one in-flight operation, one terminal result
  per item, retained prior successes, and continuation after an injected
  independent failure. Live ERT proves multi-page Collection selection plus
  create, update, pull, delete, partial-failure, and interruption-safe retry
  behavior against a real Jaunder server.
- The ordinary Jaunder verification ladder passes with the changed Emacs client
  and documentation.

## Boundaries

- No automatic or background synchronization, implicit direction choice, content
  merge, conflict override, or absence-means-delete behavior.
- No nested-file discovery, recursive directory synchronization, Markdown or
  HTML publishing, or change to the configured-root ownership model.
- No bulk AtomPub endpoint, server-side reconcile operation, or concurrent
  request execution. The only server contract change is durable per-User
  create-key replay; all Post and Media operations remain ordinary AtomPub
  Member and Collection requests.
- No Media-library management, Local Media Copy cleanup, user/configuration/App
  Password administration, deletion purge, or Post restoration.
