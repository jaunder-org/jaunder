# Issue #1183 — Explicit AtomPub Post deletion from Emacs

## Outcome

An Emacs user can explicitly delete the remote Post represented by the visited
Org file. A successful conditional AtomPub deletion removes that file and its
buffer; every non-success path leaves both intact.

## Load-bearing decisions

- Deletion is an explicit command on one visited Org file. Reconciliation and
  ordinary local file deletion never imply remote deletion.
- The command requires a visited file, a valid numeric `JAUNDER_ID`, and a
  strong quoted, non-weak `JAUNDER_SYNCED` ETag before it prompts or performs
  network I/O.
- Missing, malformed, weak, or unquoted sync markers are local validation
  failures. They cannot fall back to an unconditional delete.
- The active directory configuration determines the User and AtomPub Collection
  through the existing blog-selection boundary. No separate Blog domain entity
  or configuration path is introduced.
- After interactive confirmation, the Protocol Client sends a conditional
  AtomPub DELETE for the Member identified by `JAUNDER_ID`, using
  `JAUNDER_SYNCED` as `If-Match`.
- Cancellation performs no network or local mutation.
- HTTP `204 No Content` is the sole success status. Only after receiving it may
  the client delete the visited file and kill its buffer.
- HTTP `404 Not Found` means there is no active Member at that endpoint; it does
  not authorize local deletion. HTTP `412 Precondition Failed` reports stale
  synchronization state and likewise preserves local state.
- Every other HTTP status and every transport failure surfaces through the
  existing Emacs transport contract and preserves the file and buffer.
- The server retains its Deleted Post tombstone under the established Post
  lifecycle; this feature promises removal from active surfaces, not erasure.

## Acceptance

- A valid synchronized Org file, confirmed by the user and answered with `204`,
  is deleted remotely, removed from disk, and no longer has a live buffer.
- Cancellation leaves the file byte-identical, keeps its buffer live, and sends
  no DELETE request.
- A missing visited file, invalid `JAUNDER_ID`, or missing, malformed, weak, or
  unquoted `JAUNDER_SYNCED` fails before confirmation and network I/O while
  preserving local state.
- `404`, `412`, any other non-`204` response, and transport failure each surface
  their status or error while preserving local bytes and the visited buffer.
- Pure ERT proves validation ordering, confirmation behavior, conditional
  request construction, and every response branch.
- Live ERT against the real Jaunder server proves successful two-sided deletion
  and proves that stale or missing ETags preserve the local file and buffer.

## Boundaries

- No inventory, pull, reconciliation, publish, overwrite, bulk deletion,
  hand-deletion propagation, retry policy, or automatic conflict resolution.
- No new transport, configuration, authentication, Post lifecycle, or domain
  vocabulary decision; the accepted Emacs and AtomPub contracts remain in force.
