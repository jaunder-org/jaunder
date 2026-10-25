# Confirm merge before reporting terminal dequeue

## Outcome

When PR observation has seen a merge-queue entry disappear for the same PR head,
it reports `merged` if a bounded confirmation establishes that the PR merged,
rather than prematurely returning `dequeued`. A genuine same-head dequeue
remains distinct; inability to confirm remains an explicit observation failure,
not an inferred merge.

## Approach and tradeoffs

Use the existing PR observation/source seam and terminal classification flow.
Before committing a `dequeued` terminal verdict, obtain one fresh read of the
same PR. Confirm dequeue only when that read has the exact previously queued
head SHA, PR state `OPEN`, and queue membership false. Explicit `MERGED` state
takes precedence and carries its merge timestamp/commit; `CLOSED` remains
`closed-unmerged`. If the fresh read has a different head, discard the old
queue-history attribution and classify that snapshot normally under existing
rules. Validate the confirmation's PR identity against the requested PR, and
validate the raw response's PR state and queue-membership fields before
classification; missing, malformed, or unrecognized required
identity/state/queue fields are malformed snapshot responses and must enter the
watcher's existing poll-error/strike/deadline policy. Do not rely on snapshot
defaults (such as `OPEN` or not-in-queue) to turn absent or unknown raw fields
into confirmation. Keep existing typed incomplete Actions-classification
handling separate and unchanged; it is not the policy for malformed PR
identity/state/queue fields. Neither API errors nor malformed required fields
may become `dequeued` or inferred `merged` without the predicate. Keep the
read-only observer / arming separation; do not re-enqueue. This adds one
confirmation request per candidate-dequeue decision (subsequent attempts, if
any, are governed by existing polling policy) and cannot prove consistency
semantics of GitHub's API beyond its returned snapshot.

## Acceptance

- A queued same-head PR whose confirmation read reports `MERGED` returns
  terminal `merged`, with the returned merge metadata.
- `dequeued` requires the confirmation snapshot to match the queued head SHA,
  report `OPEN`, and report not-in-queue.
- A `CLOSED` confirmation returns `closed-unmerged`; a different head clears old
  queue attribution and that snapshot follows ordinary classification.
- Confirmation validates returned PR identity against the requested PR and
  validates raw PR state and queue-membership fields. Missing, malformed, or
  unrecognized required identity/state/queue fields are malformed snapshot
  responses routed through the existing poll-error/strike/deadline policy—not
  defaulted into confirmation and not described as typed incomplete Actions
  evidence. Existing typed incomplete Actions-classification behavior remains
  separate and unchanged. Neither API errors nor malformed required fields can
  yield `dequeued` or inferred `merged`.
- Raw-response regression fixtures cover missing, malformed, and unrecognized
  required PR identity, state, and queue-membership fields, and demonstrate they
  follow the poll-error/strike/deadline path.
- Existing ejection, required-check failure, deadline, and `pr watch` read-only
  / `pr land` arming behavior remain intact.
- Verify through existing offline PR-source/fake-clock tests, including the real
  watcher/land caller path. No live merge or queue mutation is part of proof.

## Boundaries

No changes to merge-queue configuration, approval policy, automatic re-enqueue,
queue framework, broad retry behavior, or unrelated PR outcomes. Do not assert
why GitHub returned discrepant state or that this is a race. No implementation
or tests before explicit spec/scope approval.
