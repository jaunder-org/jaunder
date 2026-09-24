# ADR-0213: Uncapped exact Post management mutations

- Status: accepted
- Date: 2026-09-24
- Issue: [#1625](https://github.com/jaunder-org/jaunder/issues/1625)

## Context

Manage Posts lets a User confirm an immutable Management Selection Snapshot and
then synchronously replace the complete Audience Selection or delete every Post
in that snapshot. “Select all matching” is only truthful if execution targets
every confirmed match; silently truncating at a page or cap would misrepresent
the destructive scope. A hard cap without inverse selection would also leave no
way to express “all except these few Posts.”

[ADR-0092](0092-sqlite-bounded-write-lock-occupancy.md) requires batches on the
SQLite path to be capped by construction. The synchronous, atomic,
all-or-nothing, uncapped product contract for this operation conflicts with that
general rule: write-lock occupancy necessarily grows with the exact confirmed
selection. Database bind limits are a separate implementation constraint and
must not become a hidden product cap.

## Decision

Exact bulk Post management mutations are a narrow exception to ADR-0092's
bounded-batch rule. They may execute an uncapped Management Selection Snapshot
inside one write-first transaction so that validation, complete Post Revision
capture, mutation, and feed-event enqueue either all commit or all roll back.

The exception does not permit per-row writes or unbounded SQL parameter lists.
Every read and write over snapshot members uses set-based statements whose input
is partitioned into fixed-size, backend-safe bind batches. Chunking may increase
the number of statements inside the one transaction, but may not split the
operation into independently committed subsets. Selection resolution remains a
read outside the write transaction; execution validates the exact Post IDs and
mutation versions again after acquiring the write scope.

This exception applies only to the two Manage Posts operations introduced by
issue #1625. Other SQLite write paths remain governed by ADR-0092.

## Consequences

- “Select all matching” remains literal and the operation remains synchronous,
  atomic, and all-or-nothing at every selection size the deployment can store.
- SQL bind counts remain bounded even though the logical operation is uncapped.
- A very large operation can hold SQLite's single writer lock longer than
  ADR-0092 normally permits. Operators receive correctness rather than partial
  progress; ordinary concurrent writers may wait or fail at the existing busy
  timeout.
- The implementation must retain dual-backend tests for batching, rollback,
  stale snapshots, semantic no-ops, revisions, and feed-event atomicity.
- A future result-aware or inverse-selection contract may introduce an honest
  cap or background execution model. Until then, a cap is not an implementation
  detail and must not be added silently.
