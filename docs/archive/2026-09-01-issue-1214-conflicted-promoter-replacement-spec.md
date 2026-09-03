# Issue #1214: Replace conflicted promoter attempts from fresh main

## Outcome

The serialized ADR-promoter controller recovers from incomplete controller-owned
state by cleaning it up with an exact-SHA lease and regenerating from fresh
`main`. It never rebases, force-updates, or edits a promoter PR's head, diff, or
content in place.

## Authority and events

- Only `Generate` events—`push` to `main` and `workflow_dispatch`—may clean up
  and regenerate. They share the non-canceling `adr-promoter-generate`
  concurrency group. A `dequeued` event remains exact-head auto-merge re-arm
  recovery only; it never closes, deletes, or regenerates.
- `automation/adr-promoter` is controller-owned. Repository administrators are
  trusted not to mutate it during a run. The controller identifies an open
  promoter PR by its expected stable ref, `main` base, App author, and exact
  head SHA; a mismatched or ambiguous PR/ref state fails closed.
- Pull Requests write covers close/create/arm; Contents write covers the Git
  ref operations. Existing App permissions and workflow triggers are unchanged.

## Generate state machine

Generate reads the open exact promoter PR and the stable ref. The first matching
case wins:

1. **Open exact promoter PR; stable ref absent.** This is interrupted controller
   retirement. Close that exact PR, verify it closed at its observed head, then
   regenerate.
2. **Open PR with a different stable ref.** Fail closed.
3. **Open exact promoter PR and exact stable ref.** Classify it as follows:
   - On positive conflict evidence, lease-delete the ref at the exact observed
     head, verify absence, close and verify that exact PR, then regenerate.
     Evidence requires GitHub `mergeable = CONFLICTING` and
     `mergeStateStatus = DIRTY`, the generated head's sole parent being a strict
     ancestor of current `main`, and a local merge-tree conflict over the exact
     fetched objects. Unknown/blocked mergeability, queue delay, pending or
     failed checks, or an external close never establish conflict evidence.
   - An armed or queued PR remains `Existing`.
   - An unarmed PR with failed required checks remains visible and unarmed.
   - An unarmed PR with pending or green required checks is armed, then verified
     at its exact head.
4. **No open promoter PR and stable ref present.** This is incomplete controller
   publication. Lease-delete the exact observed ref and regenerate.
5. **No open promoter PR and no stable ref.** Generate normally.

A lease is used only to delete the exact observed ref:
`--force-with-lease=refs/heads/automation/adr-promoter:<H>` with a deletion
refspec. A changed head fails without mutation. REST/GraphQL ref deletion and
force-updating the branch are forbidden. The controller performs exact
postcondition reads after deletion, close, push, create-or-read, and arm so an
ambiguous transport result is classified by observed state before retry.

## Fresh generation

Each generation fetches fresh `main`, runs the existing promotion mutation, and
creates a candidate from that tip. It publishes only with a non-force push,
verifies the exact remote SHA, creates or reads only the exact matching PR, arms
auto-merge, and verifies the exact head is armed or queued.

A later Generate event handles a subsequent `main` advance through the same
state machine; it replaces an existing promoter only on new positive conflict
evidence. Duplicate Generate events converge under serialization: a later run
observes the exact armed or queued promoter as `Existing`. A deterministic
required-check failure remains visible rather than being retried or replaced
without new positive conflict evidence.

## Operator guidance

Failure visibility is PR/ref state and workflow output. Operators diagnose that
visible state and rerun the authorized controller; they never manually close,
delete, rebase, force-update, promote, or otherwise alter a promoter attempt.

## Verification

- Controller tests cover Generate classifications, exact-head re-arm, positive
  conflict precedence, failed-check preservation, duplicate convergence, and
  recovery from interrupted cleanup or publication.
- Temp Git repositories prove exact-object conflict classification, exact-SHA
  lease-deletion success and changed-head refusal, and non-force publication.
- Existing promotion tests continue to prove fresh-main numbering, complete
  pending-draft inclusion, deterministic slug order, status/citation rewrite,
  and index regeneration.

## Non-goals

- Replacing a merely pending, queued, unknown, blocked, or failed-check promoter
  without positive conflict evidence.
- Retrying deterministic CI failures unattended.
- Changing ADR allocation/promotion mechanics, merge-queue policy, App
  permissions, or workflow triggers.
- Editing `CONTEXT.md` or generated `docs/README.md`.
